//! The lowering: a store's syntax trees to IR text. The IR is the
//! meaning (zero.md section 17 is the table this file implements), so
//! every construct here emits the form the section names and nothing
//! cleverer, in probe's structured form, readable by a person and
//! accepted by `probe parse` — which is the oracle for this front end.
//!
//! Names: an open-syntax name mangles to its words joined by `_`;
//! temporaries are `_1`, `_2`, ... (a zero name may not start with `_`);
//! a variable assigned again is `name_2`, `name_3`, ... so the IR stays
//! in SSA and a reader can follow the versions.
//!
//! Types are strict inside a body — two operands share a type or one is
//! a literal — and the tower of abstract numbers binds only at a call:
//! an `int32` argument fits a `number` parameter, and a `number` result
//! comes back as `int32`, which is how the IR instantiates its templates.
//! A struct is the IR's `struct`, an enumeration an unsigned integer
//! with named constants, a string a `u8[]` view of bytes.

use super::lex::{self, Error};
use super::store::{Case, Expect, Mark, Store};
use super::syntax::{Arg, Decl, Expr, ExprKind, FnDecl, Init, LoopInto, NamePart, Part, Stmt, TypeKind};
use std::collections::HashMap;
use std::fmt::Write;

/// a zero type, as the lowering sees it
#[derive(Clone, Debug, PartialEq)]
pub enum Ty {
    Bool,
    /// a number, by its IR name: `int`, `u8`, `f32`, `number`, ...
    Num(String),
    /// a character (section 4, question 44): a type of its own, distinct
    /// from `uint8`, so that `char$ << 42` asks for human-readable text
    /// where `uint8$ << 42` would ask for a serialisation. It is a `u8`
    /// in the IR, takes comparisons and conversions and no arithmetic,
    /// and `string` is `char$`
    Char,
    /// a stream `T$` (section 9, log 38): the IR's `T$`, a reader's view
    /// of a ring, whether its items arrive over time or are all present
    /// (a sequence); a `string` is `u8$`; a stream of a struct is one
    /// ring whose item is the struct (log 88, question 43)
    Stream(Box<Ty>),
    /// a declared struct, by name
    Struct(String),
    /// a declared enumeration, by name
    Enum(String),
    /// no value: a function with no results
    None,
}

impl Ty {
    fn ir(&self) -> String {
        match self {
            Ty::Bool => "u1".into(),
            Ty::Char => "u8".into(),
            Ty::Num(n) => n.clone(),
            Ty::Stream(e) => format!("{}$", e.ir()),
            Ty::Struct(n) | Ty::Enum(n) => n.clone(),
            Ty::None => String::new(),
        }
    }

    /// the spelling a method's IR name is mangled from: the IR's type,
    /// except that a `char` says so, so that a method over `char$` and
    /// one over `uint8$` are two functions in the IR
    fn mangled(&self) -> String {
        match self {
            Ty::Char => "char".into(),
            Ty::Stream(e) if **e == Ty::Char => "char$".into(),
            t => t.ir(),
        }
    }

    fn string() -> Ty {
        Ty::Stream(Box::new(Ty::Char))
    }

    /// the element type of a stream, or none
    fn elem(&self) -> Option<&Ty> {
        match self {
            Ty::Stream(e) => Some(e),
            _ => None,
        }
    }

    /// a stream of numbers or enumerations: one the sequence words
    /// (map, zip, reduce, `frame`) read as one view; a stream of structs
    /// is not
    fn items(&self) -> Option<&Ty> {
        match self {
            Ty::Stream(e) if matches!(e.as_ref(), Ty::Num(_) | Ty::Enum(_) | Ty::Char) => Some(e),
            _ => None,
        }
    }

    /// is this an abstract number, one the tower binds at a call?
    fn abstract_name(&self) -> Option<&str> {
        match self {
            Ty::Num(n) if matches!(n.as_str(), "number" | "scalar" | "int" | "uint" | "float" | "fixed" | "unit" | "sunit" | "rational" | "decimal") => Some(n),
            _ => None,
        }
    }
}

/// zero's spelling of a builtin number type to the IR's (section 4)
fn builtin_type(name: &str) -> Option<Ty> {
    let ir = match name {
        "bool" => return Some(Ty::Bool),
        "string" => return Some(Ty::string()),
        "int" | "uint" | "float" | "number" | "scalar" | "fixed" | "unit" | "sunit" | "rational" | "decimal" | "time" => name.to_string(),
        "int8" => "i8".into(),
        "int16" => "i16".into(),
        "int32" => "i32".into(),
        "int64" => "i64".into(),
        "int128" => "i128".into(),
        "char" => return Some(Ty::Char),
        "uint8" | "byte" => "u8".into(),
        "uint16" => "u16".into(),
        "uint32" => "u32".into(),
        "uint64" => "u64".into(),
        "uint128" => "u128".into(),
        "float16" => "f16".into(),
        "bfloat16" => "bf16".into(),
        "float32" => "f32".into(),
        "float64" => "f64".into(),
        _ => return None,
    };
    Some(Ty::Num(ir))
}

/// does an argument of type `arg` fit a parameter of type `param`? Its
/// own type does, and so does an abstract type above it in the tower
/// (ssa.md, *Abstract numeric types*): the call instantiates the
/// template with the argument's type
/// a type with no abstract name in it, which the IR takes as it is
/// rather than as a template's parameter (log 52): a widthed number,
/// `bool`, a struct, an enumeration, or a stream of one
fn is_concrete(t: &Ty) -> bool {
    match t {
        Ty::Num(n) => n == "bf16" || (n.len() > 1 && matches!(&n[..1], "i" | "u" | "f") && n[1..].parse::<u32>().is_ok()),
        Ty::Bool | Ty::Char | Ty::Struct(_) | Ty::Enum(_) => true,
        Ty::Stream(e) => is_concrete(e),
        Ty::None => false,
    }
}

fn fits(arg: &Ty, param: &Ty) -> bool {
    if arg == param {
        return true;
    }
    // a stream fits by its element (log 59): an `int64$` reaches a
    // method over `int x$`, the IR instantiating the template
    if let (Ty::Stream(a), Ty::Stream(p)) = (arg, param) {
        return fits(a, p);
    }
    let (Ty::Num(a), Ty::Num(p)) = (arg, param) else {
        return false;
    };
    let width = |s: &str| s[1..].parse::<u32>().is_ok();
    let is_float = |s: &str| matches!(s, "f16" | "bf16" | "f32" | "f64" | "float");
    let is_scalar = |s: &str| is_float(s) || matches!(s, "fixed" | "unit" | "sunit" | "rational" | "decimal" | "scalar" | "time");
    let is_int = |s: &str| s == "int" || (s.starts_with('i') && width(s));
    let is_uint = |s: &str| s == "uint" || (s.starts_with('u') && width(s));
    match p.as_str() {
        "number" => true,
        "scalar" => is_scalar(a),
        "int" => is_int(a),
        "uint" => is_uint(a),
        "float" => is_float(a),
        _ => false,
    }
}

/// The type two concrete number types compute in (question 1, log 37;
/// question 21, log 46): the type that holds every value of both
/// exactly where one exists (`wider_exact`), else the widest type of
/// the family that holds them best — a float and an integer no float's
/// significand holds (`int64` and wider) compute in `float64`,
/// rounding above 2^53, and `int128` with `uint128` in `int128`,
/// modulo its range. None for an abstract type (`int` has no width
/// here) or a library type (`time`, `rational`, ...): those convert
/// only by `T(x)`
fn wider(a: &Ty, b: &Ty) -> Option<Ty> {
    if let Some(t) = wider_exact(a, b) {
        return Some(t);
    }
    let (Ty::Num(x), Ty::Num(y)) = (a, b) else {
        return None;
    };
    let float = |s: &str| matches!(s, "f16" | "bf16" | "f32" | "f64");
    let int = |s: &str| s.len() > 1 && s.starts_with(['i', 'u']) && [8, 16, 32, 64, 128].contains(&s[1..].parse::<u32>().unwrap_or(0));
    if (float(x) && int(y)) || (int(x) && float(y)) {
        return Some(Ty::Num("f64".into()));
    }
    if int(x) && int(y) {
        return Some(Ty::Num("i128".into()));
    }
    None
}

/// The type two concrete number types compute in without loss, the
/// wider or more accurate of them (question 1, log 37): the wider of
/// two signed or two unsigned widths; a signed type wider than an
/// unsigned one, or the signed type twice the unsigned width; the wider
/// of two floats, `float32` for `bfloat16` with `float16`; a float and
/// an integer compute in the float when it holds every value of the
/// integer exactly (`float32` holds `int16`, `float64` holds `int32`),
/// else in the smallest float that does, and there is none for
/// `int64` and wider. None where nothing holds both exactly, and
/// always for an abstract type (`int` has no width here) or a library
/// type (`time`, `rational`, ...)
fn wider_exact(a: &Ty, b: &Ty) -> Option<Ty> {
    if a == b {
        return Some(a.clone());
    }
    let (Ty::Num(x), Ty::Num(y)) = (a, b) else {
        return None;
    };
    // an integer's class and width, or a float's class and its width
    let class = |s: &str| -> Option<(char, u32)> {
        match s {
            "f16" => Some(('f', 16)),
            "bf16" => Some(('b', 16)),
            "f32" => Some(('f', 32)),
            "f64" => Some(('f', 64)),
            _ => {
                let c = s.chars().next()?;
                let n = s[1..].parse::<u32>().ok()?;
                if matches!(c, 'i' | 'u') && [8, 16, 32, 64, 128].contains(&n) {
                    Some((c, n))
                } else {
                    None
                }
            }
        }
    };
    let ((ca, na), (cb, nb)) = (class(x)?, class(y)?);
    // the bits a float holds exactly, and the bits an integer needs
    let holds = |c: char, n: u32| match (c, n) {
        ('f', 16) => 11,
        ('b', 16) => 8,
        ('f', 32) => 24,
        ('f', 64) => 53,
        _ => 0,
    };
    let needs = |c: char, n: u32| if c == 'i' { n - 1 } else { n };
    let t = match ((ca, na), (cb, nb)) {
        (('i', n), ('i', m)) => format!("i{}", n.max(m)),
        (('u', n), ('u', m)) => format!("u{}", n.max(m)),
        (('i', n), ('u', m)) | (('u', m), ('i', n)) => {
            let k = if n > m { n } else { n.max(2 * m) };
            if k > 128 {
                return None;
            }
            format!("i{}", k)
        }
        (('f', n), ('f', m)) => format!("f{}", n.max(m)),
        (('b', _), ('f', m)) | (('f', m), ('b', _)) => format!("f{}", m.max(32)),
        ((fc @ ('f' | 'b'), fw), (ic @ ('i' | 'u'), iw)) | ((ic @ ('i' | 'u'), iw), (fc @ ('f' | 'b'), fw)) => {
            let need = needs(ic, iw);
            if holds(fc, fw) >= need {
                if fc == ca && fw == na { x.clone() } else { y.clone() }
            } else if need <= 24 {
                "f32".into()
            } else if need <= 53 {
                "f64".into()
            } else {
                return None;
            }
        }
        _ => return None,
    };
    Some(Ty::Num(t))
}

/// is a conversion from `from` to `to` implied, `to` being what the
/// pair computes in? Exact where `wider_exact` says so, else lossy
/// (log 46), which `widen_to` notes in the IR
fn widens(from: &Ty, to: &Ty) -> bool {
    from != to && wider(from, to).as_ref() == Some(to)
}

/// how an implied conversion loses bits, for the note in the IR: none
/// where it is exact, else the rounding or the wrap
fn loses(from: &Ty, to: &Ty) -> Option<String> {
    if wider_exact(from, to).as_ref() == Some(to) {
        return None;
    }
    Some(match to {
        Ty::Num(t) if t == "f64" => format!("{} into {}: rounded above 2^53", zero_ty(from), zero_ty(to)),
        _ => format!("{} into {}: modulo the range", zero_ty(from), zero_ty(to)),
    })
}

/// a function the store defines, as the lowering knows it
#[derive(Clone, Debug)]
pub struct FnInfo {
    /// the mangled name a call is matched by
    pub key: String,
    /// the IR function's name: the key, except for an operator on a
    /// declared type (`add_Vec`) and a method after the first of its
    /// name, which the IR names by its parameter types (log 52)
    pub ir: String,
    /// the name the definition is written under: the key when the
    /// name's methods form an IR method set (log 52), else the IR name
    pub plain: String,
    /// the name's methods are all concrete and form one method set in
    /// the IR, every definition under the key: a call on the key with a
    /// literal typed `int` or `float` is resolved by the policy
    pub in_set: bool,
    pub parts: Vec<NamePart>,
    pub params: Vec<(String, Ty)>,
    pub results: Vec<(String, Ty)>,
    pub feature: String,
    /// a task (log 25): declared with `<<`, its result a stream it
    /// produces and its `$` parameters streams it reads; in the IR its
    /// output's reader comes first and `__hz` last, and it returns its
    /// stream parameters moved on
    pub task: bool,
    /// the features that define it, innermost first (log 28): one for a
    /// function defined once; more for a chain of redefinitions, whose
    /// bodies are `key__feature` and whose links `key` and
    /// `key__before_feature` gate on each feature's `enabled`
    pub chain: Vec<String>,
    /// a platform function (log 31): the kinds of place its bodies are
    /// for — the IR's targets, or `ir` for a body in the IR that serves
    /// every target — and None for a function with a zero body
    pub platform: Option<Vec<String>>,
}

/// the refusal of a program that writes its input (question 35)
const INPUT_REFUSED: &str = "'in$' is the input device: a program reads it and never writes it or ends it. Input comes from the platform alone, which under the runner is a case's `with in \"text\"`; a program that makes its own arrivals pushes them into a stream of its own";

/// the kinds of place a platform body may name in this milestone
const KINDS: [&str; 5] = ["ir", "arm64", "riscv64", "wasm32", "air"];

/// a wiring at feature scope (log 25): one task call the scheduler runs
/// into a feature-scope stream
struct Node {
    info: FnInfo,
    /// the output stream variable; none for a sink (log 57)
    out: Option<String>,
    /// the arguments in the parameters' order: a stream parameter's is
    /// the name of a feature-scope stream
    args: Vec<Expr>,
    hz: i64,
    feature: String,
    file: String,
    /// the wiring as written, for the IR's comment
    text: String,
}

/// a declared type
#[derive(Clone, Debug)]
enum TypeInfo {
    /// fields: name, type, default (a literal's text)
    Struct(Vec<(String, Ty, Option<String>)>),
    Enum(Vec<String>),
}

/// a feature-scope variable: a field of the store's context struct
/// (log 16), with the columns section 5 declares
#[derive(Clone, Debug)]
struct FVar {
    name: String,
    ty: Ty,
    /// `user`, `static`, `device` or `group`
    scope: String,
    /// `last`, or the word after `merge`
    merge: String,
    feature: String,
}

pub struct Lowered {
    pub ir: String,
    pub funcs: Vec<FnInfo>,
    /// the features, in composition order
    pub features: Vec<String>,
    /// the product's mark per feature (log 71), the static-off ones
    /// included though they are not among the features
    pub marks: HashMap<String, Mark>,
}

/// The rounds of dispatch on a literal (log 36, 47, 52): first as its
/// own type (`3` an `int`, `2.5` a `float`), then as a concrete width
/// the policy may take, tried per width, then fitting any number type
#[derive(Clone, Copy, PartialEq, Debug)]
enum Round {
    Own,
    Product,
    Any,
}

/// the abstract `int`, a literal's own type
fn int_ty() -> Ty {
    Ty::Num("int".into())
}

/// the abstract `float`, a decimal literal's own type
fn float_ty() -> Ty {
    Ty::Num("float".into())
}

/// the widths the policy may give `int` and `float` (log 52)
const WIDTHS: [u32; 2] = [32, 64];

/// what the runner calls for a case: the IR function, its integer
/// arguments, how many results it has, what the case expects, and the
/// context its line names (log 43, 44): each feature and whether it is
/// on, checked to be the store's; the runner turns it into setter calls
pub struct Call {
    pub func: String,
    pub args: Vec<i64>,
    pub nrets: usize,
    pub expect: Expect,
    pub context: Vec<(String, bool)>,
    /// the bytes the runner pushes into `in$` before the start (log 62)
    pub input: Vec<u8>,
}

/// the IR every store gets: the arena, the clock, the two functions
/// the runner reads `out$` back with (log 57), the push any ring takes
/// (log 38), and `__str` (a string literal's view). `__zero_reset`,
/// which the runner calls before each case, is generated per store
/// (log 16). Formatting is the platform feature's, in zero (log 59)
const PRELUDE: &str = r#"
; an empty string's bytes
data __nul: array(u8, 1)

; the arena every sequence is carved from, emptied before every case
data __heap: array(u8, 65536)
data __arena: array(i64, 3)

; the store's virtual clock (log 63): integer ticks on a clock of a
; million a second, the bootstrap's step; zero at every case, moved on
; a step by a push into a stream with a rate and by a rated task after
; each of its pushes, and by nothing else; a push into a stream without
; a rate is stamped with it. Exact time is the boundary's: `x$ at (t)`
; and `position` convert once, in the library
data __clock: array(i64, 1)
; the scheduler is running: a push from inside a task does not start it again
data __running: array(i64, 1)

fn __now() -> i64
    p: ptr = addr __clock
    k: i64 = load p
    ret k

; a task wired at a rate, after each push: the clock reaches the next
; period, in whole ticks, as a rated ring steps
fn __sleep(hz: i64)
    rated: u1 = cmp.gt hz, 0
    if rated
        p: ptr = addr __clock
        c: i64 = load p
        d: i64 = div 1000000, hz
        c2: i64 = add c, d
        __wait(c2)
    ret

; the output device (section 15, question 45, log 87): `out$` looks like
; a stream, but a push into it writes a character to the place and
; stores nothing, so these two are the platform's write — one character,
; and a block of them. There is no capacity, because there is no
; storage. Under the runner the place is the capture below, which
; `__out_len` and `__out_byte` read back; a real platform's body per
; kind of place — a UART store on the boards, a write to the console
; under an OS, the browser's log — is a `platform <target>` block on
; each of these, beside the `ir` body, and is milestone 1's
data __out: array(u8, 65536)
data __out_n: array(i64, 1)

fn __out_ch(c: u8)
    q: ptr = addr __out_n
    n: i64 = load q
    p: ptr = addr __out
    store c, p, n, 1
    n2: i64 = add n, 1
    store n2, q
    ret

fn __out_block(v: u8[])
    k: i64 = len v
    q: ptr = addr __out_n
    n: i64 = load q
    p: ptr = addr __out
    r: ptr(u8) = cast p
    all: u8[] = pack r, 65536, 1
    d: u8[] = view all, n, k
    copy d, v
    n2: i64 = add n, k
    store n2, q
    ret

; what the device was given, read back by the test runner: the capture
; is the runner's, not the store's
fn __out_len() -> i64
    q: ptr = addr __out_n
    n: i64 = load q
    ret n

fn __out_byte(i: i64) -> u8
    p: ptr = addr __out
    b: u8 = load p, i, 1
    ret b

; when it was written (question 55, fm3 log 95): each time the virtual
; clock moves, `__wait` stores the time it reached at the place in the
; output where it begins to apply, a word for each byte of the capture
; and one for its end, indexed by how many bytes the device had been
; given. A word that is zero says the clock did not move there. The
; runner reads a word for each byte it read, and one more, for a case
; that asserts on time
data __out_t: array(i64, 65537)

fn __out_mark(i: i64) -> i64
    p: ptr = addr __out_t
    w: i64 = load p, i, 8
    ret w

; the input device (section 15, log 62, question 45): a character
; arriving, which under the runner is a case's `with in "text"`. Its
; ring is the device's own lookahead, sized by the runner, which a
; consumer wired to `in$` reads with `peek`, `advance` and `count`
fn __in_ch(c: u8)
    k: __ctx = load _this
    s: u8$ = get k, in
    __push(s, c)
    ret

; a string literal's bytes: a view of n bytes at p, which `__copy_u8` makes a stream
fn __str(p: ptr, n: i64) -> u8[]
    q: ptr(u8) = cast p
    v: u8[] = pack q, n, 1
    ret v

"#;

/// `__zero_reset`'s lines that forget the last case's marks: the table
/// cleared as far as that case wrote, before its count of bytes is
/// zeroed (fm3 log 95)
const MARKS_RESET: [&str; 10] = [
    "mo: ptr = addr __out_n",
    "mn: i64 = load mo",
    "mt: ptr = addr __out_t",
    "loop(mi: i64 = 0) bound 65536",
    "    store 0: i64, mt, mi, 8",
    "    md: u1 = cmp.ge mi, mn",
    "    if md",
    "        break",
    "    mi2: i64 = add mi, 1",
    "    continue mi2",
];

/// the clock reaches a time (log 77): on the virtual clock it jumps
/// there, the suite running as fast as it can
const VIRTUAL_CLOCK: &str = r#"
; the clock reaches t (log 77): the virtual clock jumps there, and marks
; the place in the output where that time begins (fm3 log 95)
fn __wait(t: i64)
    p: ptr = addr __clock
    c: i64 = load p
    m: i64 = max(c, t)
    store m, p
    q: ptr = addr __out_n
    n: i64 = load q
    a: ptr = addr __out_t
    store m, a, n, 8
    ret
"#;

/// ... and on the real clock it waits on the machine's counter: the
/// board's `now()` and `hz()` as a `platform arm64` body, read at the
/// reset into `__base`; a path without the rule fails the check rather
/// than spin forever
const REAL_CLOCK: &str = r#"
; the real clock (log 77): the machine's counter, read at the reset and
; whenever the store's clock must reach a time
data __base: array(i64, 1)

fn __counter() -> i64
    z: u1 = const 0
    check z
    ret 0
platform arm64
    __counter() -> i64
        mrs r, cntpct_el0

fn __counter_hz() -> i64
    z: u1 = const 0
    check z
    ret 0
platform arm64
    __counter_hz() -> i64
        mrs r, cntfrq_el0

; microseconds since the reset
fn __real_now() -> i64
    c: i64 = __counter()
    p: ptr = addr __base
    b: i64 = load p
    e: i64 = sub c, b
    m: i64 = mul e, 1000000
    hz: i64 = __counter_hz()
    us: i64 = div m, hz
    ret us

; the clock reaches t (log 77): the real one is waited for
fn __wait(t: i64)
    loop()
        r: i64 = __real_now()
        done: u1 = cmp.ge r, t
        if done
            break
        else
            continue
    p: ptr = addr __clock
    c: i64 = load p
    m: i64 = max(c, t)
    store m, p
    ret
"#;

/// a push into any stream (log 38): stamped with the clock on a ring
/// that keeps ticks, the next sample on one that does not — or, in a
/// store where no ring keeps ticks (log 73), the plain push itself
const PUSH_BRANCHED: &str = r#"
; a push into any stream (log 38): stamped with the clock on a ring
; that keeps ticks, the next sample on one that does not. Over `any`,
; since a ring may hold a struct (log 88)
fn __push(s: any$, v: any)
    r: ptr = get s, ring
    step: i64 = load r, 40
    regular: u1 = cmp.gt step, 0
    if regular
        push(s, v)
    else
        t: i64 = __now()
        push(s, t, v)
    ret
fn __push(s: any$, block: any[])
    r: ptr = get s, ring
    step: i64 = load r, 40
    regular: u1 = cmp.gt step, 0
    if regular
        push(s, block)
    else
        t: i64 = __now()
        push(s, t, block)
    ret
"#;
const PUSH_PLAIN: &str = r#"
; a push into any stream: no ring in this store keeps ticks (log 73)
fn __push(s: any$, v: any)
    push(s, v)
    ret
fn __push(s: any$, block: any[])
    push(s, block)
    ret
"#;
/// ... and where nothing in the store keeps history either, every
/// stream is a queue (log 89, question 47), so a push through a
/// parameter is the queue's too
const PUSH_QUEUE: &str = r#"
; a push into any stream: nothing in this store keeps history, so
; every stream is a queue (log 89)
fn __push(s: any$, v: any)
    push_queue(s, v)
    ret
fn __push(s: any$, block: any[])
    push_queue(s, block)
    ret
"#;

/// a ring's capacity, or the item count where it is larger (log 38):
/// the front end's number until residency is computed (section 9, log
/// 23); a reader more than half this behind fails the library's check
/// the context's address in a function that touches the context (fm3
/// log 110): the one name every load and store of state goes through
const THIS: &str = "_this";

const RING_ITEMS: usize = 64;
/// a block of fewer items than this is moved an item at a time (fm3
/// log 109): sixteen bytes are the smallest chunk the machines have,
/// and under a chunk the library's `copy` moves every item in its item
/// loop after setting up for chunks, 51 + 10 n operations on an arm64
/// where a plain loop is 28 + 6 n; from sixteen up a chunk at a time
/// wins, 63 against 124. The machine's count decides, not the cost
/// tool's, which goes on preferring the plain loop at every length
const FEW_ITEMS: usize = 16;
/// the bytes the platform's `out$` holds (log 57): the compiler's number,
/// where the print buffer's 4096 was, until a product says
const OUT_BYTES: usize = 4096;
/// the bytes the platform's `in$` holds (log 62): a sparse ring keeps a
/// tick per byte, and a case's input is a line
const IN_BYTES: usize = 512;

/// the clock of a stream without a rate: microsecond ticks
const CLOCK_HZ: i64 = super::store::CLOCK_HZ;

/// the copy maker's name for a storage word: `__copy_T` for a ring,
/// `__copy_timed_T` for one that stamps, `__copy_queue_T` for a queue
fn copy_infix(maker: &str) -> &'static str {
    match maker {
        "queue" => "queue_",
        "stream" => "timed_",
        _ => "",
    }
}

pub fn mangle(parts: &[NamePart]) -> String {
    let words: Vec<String> = parts
        .iter()
        .filter_map(|p| match p {
            NamePart::Word(w) => Some(w.clone()),
            NamePart::Sym(s) => Some(op_name(s).trim_start_matches("cmp.").to_string()),
            NamePart::Group => None,
        })
        .collect();
    words.join("_")
}

/// an operator's opcode in the IR (section 6: `+` on a type is `add`)
fn op_name(s: &str) -> &'static str {
    match s {
        "+" => "add",
        "-" => "sub",
        "*" => "mul",
        "/" => "div",
        "%" => "rem",
        "<" => "cmp.lt",
        ">" => "cmp.gt",
        "<=" => "cmp.le",
        ">=" => "cmp.ge",
        "==" => "cmp.eq",
        "!=" => "cmp.ne",
        // a push into a stream: `<<`'s methods are `push__u8s__int`
        // and the like (log 59)
        "<<" => "push",
        _ => "op",
    }
}

fn is_comparison(op: &str) -> bool {
    matches!(op, "<" | ">" | "<=" | ">=" | "==" | "!=")
}

/// what a body may push into, for the scheduler's graph (log 78): the
/// feature-scope streams it pushes into by name or ends, and those it
/// passes to a store function that pushes into that parameter, chased
/// through every definition a phrase may reach; a body's own parameters
/// and locals are not feature-scope names. An over-approximation
/// wherever it is unsure: a spurious edge costs the loop, never a
/// missed run
struct Pushes<'a> {
    l: &'a Lowerer,
    /// every definition of a key: its parameter names in order, and its body
    bodies: HashMap<String, Vec<(Vec<String>, &'a [Stmt])>>,
    /// per key: the feature-scope streams pushed, the parameters pushed
    known: HashMap<String, (std::collections::HashSet<String>, std::collections::HashSet<String>)>,
    /// the keys being computed: a recursion meets them and assumes the worst
    active: std::collections::HashSet<String>,
    file: String,
    /// how deep in other definitions' bodies the walk is: 0 in the
    /// block it was asked about
    depth: usize,
    /// the first place, in the block asked about, where the input device
    /// is handed to a function as a parameter that function pushes into
    /// or ends: the line, and the function (question 35, fm3 log 106)
    handed: Option<(usize, String)>,
}

type Names = std::collections::HashSet<String>;

impl<'a> Pushes<'a> {
    /// the streams a key's definitions may push into, cached
    fn of_key(&mut self, key: &str) -> (Names, Names) {
        if let Some(k) = self.known.get(key) {
            return k.clone();
        }
        if self.active.contains(key) {
            // recursive: every stream parameter of every definition
            let params: Names = self.bodies.get(key).map(|ds| ds.iter().flat_map(|(ps, _)| ps.iter().cloned()).collect()).unwrap_or_default();
            return (Names::new(), params);
        }
        self.active.insert(key.to_string());
        self.depth += 1;
        let (mut out, mut own) = (Names::new(), Names::new());
        let defs = self.bodies.get(key).cloned().unwrap_or_default();
        for (params, body) in defs {
            let bound: Names = params.iter().cloned().collect();
            let mut mine = Names::new();
            self.of(body, &bound, &mut out, &mut mine);
            own.extend(mine.into_iter().filter(|n| params.contains(n)));
        }
        self.depth -= 1;
        self.active.remove(key);
        self.known.insert(key.to_string(), (out.clone(), own.clone()));
        (out, own)
    }

    /// the streams `stmts` may push into, `bound` being the names that
    /// are not feature-scope: into `out` the feature-scope ones, into
    /// `own` the bound ones
    fn of(&mut self, stmts: &[Stmt], bound: &Names, out: &mut Names, own: &mut Names) {
        let mut bound = bound.clone();
        for s in stmts {
            match s {
                Stmt::Var(v) => {
                    self.init(v, &bound, out, own);
                    bound.insert(v.name.clone());
                }
                Stmt::Multi { vars, value, .. } => {
                    self.expr(value, &bound, out, own);
                    for p in vars {
                        bound.insert(p.name.clone());
                    }
                }
                Stmt::Assign { value, .. } | Stmt::Expr { expr: value, .. } | Stmt::Check { cond: value, .. } => self.expr(value, &bound, out, own),
                Stmt::If { cond, then, els, .. } => {
                    self.expr(cond, &bound, out, own);
                    self.of(then, &bound, out, own);
                    if let Some(e) = els {
                        self.of(e, &bound, out, own);
                    }
                }
                Stmt::Loop { vars, cond, body, .. } => {
                    let mut inner = bound.clone();
                    for v in vars {
                        self.init(v, &inner, out, own);
                        inner.insert(v.name.clone());
                    }
                    if let Some(c) = cond {
                        self.expr(c, &inner, out, own);
                    }
                    self.of(body, &inner, out, own);
                }
                Stmt::For { var, seq, body, .. } => {
                    self.expr(seq, &bound, out, own);
                    let mut inner = bound.clone();
                    inner.insert(var.clone());
                    self.of(body, &inner, out, own);
                }
                Stmt::Continue { values, .. } => values.iter().for_each(|e| self.expr(e, &bound, out, own)),
                Stmt::Break { .. } => {}
                Stmt::Push { target, items, cond, .. } => {
                    if let ExprKind::Seq(n) = &target.kind {
                        self.pushed(n, &bound, out, own);
                    }
                    items.iter().for_each(|e| self.expr(e, &bound, out, own));
                    cond.iter().for_each(|e| self.expr(e, &bound, out, own));
                }
            }
        }
    }

    /// the input device handed, in the block asked about, to `key` as a
    /// parameter it pushes into or ends
    fn hand(&mut self, n: &str, key: &str, bound: &Names, line: usize) {
        if self.depth == 0 && self.handed.is_none() && n == "in" && !bound.contains(n) && self.l.input_device(n, None) {
            self.handed = Some((line, key.replace('_', " ")));
        }
    }

    fn pushed(&self, n: &str, bound: &Names, out: &mut Names, own: &mut Names) {
        if bound.contains(n) {
            own.insert(n.to_string());
        } else if self.l.fvar(n).is_some_and(|f| matches!(f.ty, Ty::Stream(_))) {
            out.insert(n.to_string());
        }
    }

    fn init(&mut self, v: &super::syntax::VarDecl, bound: &Names, out: &mut Names, own: &mut Names) {
        match &v.init {
            Some(Init::Value(e)) => self.expr(e, bound, out, own),
            Some(Init::Construct(args)) => args.iter().for_each(|a| self.expr(&a.value, bound, out, own)),
            Some(Init::Pushes { items, cond }) => {
                items.iter().for_each(|e| self.expr(e, bound, out, own));
                cond.iter().for_each(|e| self.expr(e, bound, out, own));
            }
            None => {}
        }
    }

    fn expr(&mut self, e: &Expr, bound: &Names, out: &mut Names, own: &mut Names) {
        match &e.kind {
            ExprKind::Unit(x, _) | ExprKind::Neg(x) | ExprKind::Field(x, _) => self.expr(x, bound, out, own),
            ExprKind::List(items) => items.iter().for_each(|x| self.expr(x, bound, out, own)),
            ExprKind::Range { from, to, .. } => {
                self.expr(from, bound, out, own);
                self.expr(to, bound, out, own);
            }
            ExprKind::Bin(_, l, r) | ExprKind::Index(l, r) => {
                self.expr(l, bound, out, own);
                self.expr(r, bound, out, own);
            }
            ExprKind::IfElse(c, t, f) => {
                self.expr(c, bound, out, own);
                self.expr(t, bound, out, own);
                self.expr(f, bound, out, own);
            }
            ExprKind::Phrase(parts) | ExprKind::Existing(parts) => {
                for p in parts {
                    match p {
                        Part::Args(list) => list.iter().for_each(|a| self.expr(&a.value, bound, out, own)),
                        Part::Value(x) => self.expr(x, bound, out, own),
                        Part::Word(_) => {}
                    }
                }
                let is_var = |w: &str| bound.contains(w) || self.l.fvar(w).is_some();
                // a task call with its rate after it hands its streams on
                // as the call alone does: asked only for the device
                let unrated = match parts.as_slice() {
                    [rest @ .., Part::Word(at), Part::Args(a)] if at == "at" && a.len() == 1 && matches!(&a[0].value.kind, ExprKind::Unit(..)) => Some(rest),
                    _ => None,
                };
                if let Some(Ok((cands, args))) = unrated.map(|rest| find_methods(&self.l.funcs, rest, &is_var, &self.file, e.line)) {
                    let cands: Vec<(String, Vec<String>)> = cands.iter().filter(|c| c.task).map(|c| (c.key.clone(), c.params.iter().map(|(n, _)| n.clone()).collect())).collect();
                    for (key, params) in cands {
                        let (_, pushed) = self.of_key(&key);
                        for (i, pname) in params.iter().enumerate() {
                            if let (true, Some(Expr { kind: ExprKind::Seq(n), .. })) = (pushed.contains(pname), args.get(i)) {
                                self.hand(n, &key, bound, e.line);
                            }
                        }
                    }
                }
                match find_methods(&self.l.funcs, parts, &is_var, &self.file, e.line) {
                    Ok((cands, args)) if !cands.is_empty() => {
                        let cands: Vec<(String, Vec<String>)> = cands.iter().map(|c| (c.key.clone(), c.params.iter().map(|(n, _)| n.clone()).collect())).collect();
                        for (key, params) in cands {
                            let (theirs, pushed) = self.of_key(&key);
                            out.extend(theirs);
                            for (i, pname) in params.iter().enumerate() {
                                if pushed.contains(pname) {
                                    if let Some(Expr { kind: ExprKind::Seq(n), .. }) = args.get(i) {
                                        self.hand(n, &key, bound, e.line);
                                        self.pushed(n, bound, out, own);
                                    }
                                }
                            }
                        }
                    }
                    // a compiler word: `end x$` is what a consumer sees as more
                    _ => {
                        if let [Part::Word(w), Part::Value(Expr { kind: ExprKind::Seq(n), .. })] = parts.as_slice() {
                            if w == "end" {
                                self.pushed(n, bound, out, own);
                            }
                        }
                    }
                }
            }
            _ => {}
        }
    }
}

/// The clock of code (question 56, fm3 log 99). Code runs because a
/// clock ticked, so at each point in a plain function the front end
/// knows what the store's one clock is a whole multiple of: 0 where it
/// is exactly where the case started it, which every period divides; P
/// where it is a multiple of a period P; 1 where nothing is known, the
/// clock being whole microseconds. Where two paths meet the fact is
/// their greatest common divisor. A push into a stream of period Q
/// needs no alignment where Q divides the fact, and the pass leaves two
/// sets for the lowering, keyed by the statement itself: the pushes
/// that are on the beat, and the loops whose first statement's
/// alignment is made once before them. A statement in neither is
/// lowered as it always was, so a mistake of omission is an alignment
/// kept.
///
/// The facts are followed through plain functions: one at each
/// definition's entry, the weakest of every call that may enter it, a
/// case's among them, and one at its exit. Everything else — a task, a
/// sink, an edge's function, an operator or a `<<` method, a platform
/// function, whatever the scheduler runs — is asked one thing, whether
/// it can move the clock, and starts at nothing known. What can move
/// the clock is the list of every place this file emits `__wait`,
/// `__sleep` or a scheduler call: `step` and `align` (a push into a
/// stream with a rate), `trigger` (a push into, or the `end` of, a
/// stream a node reads), a task's sleep after a push into its own
/// output where any wiring has a rate, a rated edge's wait in
/// `lower_for_seq`, and `run_task`. A new one belongs in `moved_by`
struct Beat<'a> {
    l: &'a Lowerer,
    defs: Vec<BeatDef<'a>>,
    /// a call's key and argument count: the definitions it may enter
    by_call: HashMap<(String, usize), Vec<usize>>,
    /// per definition, what its body may do to the clock
    evs: Vec<Vec<Ev>>,
    moves: Vec<bool>,
    /// can anything the scheduler runs move the clock?
    nodes_move: bool,
    /// can an operator or a `<<` method? Then every push and every
    /// operator may, unseen, and the pass is off
    sym_moves: bool,
    /// has the store an operator or a method of its own? A loop's
    /// `while` is then not asked twice
    user_syms: bool,
    entry: Vec<i64>,
    exit: Vec<i64>,
    /// the definitions that can reach themselves: they leave nothing known
    looped: Vec<bool>,
    /// the loops being walked: the facts at their `break`s and at their
    /// `continue`s
    loops: Vec<(Option<i64>, Option<i64>)>,
    /// the facts have stopped changing: the choices are written down
    record: bool,
    need: HashMap<usize, bool>,
    hoist: HashMap<usize, Option<i64>>,
}

/// a definition the pass knows: a function of a feature, or an edge's
struct BeatDef<'a> {
    fd: &'a FnDecl,
    key: String,
    arity: usize,
    /// its feature's place in the composition order
    at: usize,
    /// a plain function, whose facts are followed
    plain: bool,
    /// an operator or a `<<` method, and whether it is the store's own
    sym: bool,
    user: bool,
    /// a task, a sink or a stored edge's function: the scheduler's
    task: bool,
}

/// what a body may do to the clock, read off the syntax
enum Ev {
    /// a push into the stream of this name
    Push(String),
    /// an operator, or a push's dispatch: a method of the store's may run
    Op,
    /// `end x$`: the scheduler may run
    End(String),
    /// a phrase the pass cannot name, or a task run at a rate
    Unknown,
    /// a call that may enter these definitions
    Call(Vec<usize>),
    /// the stream of this name is handed to something that may push into
    /// it with no trigger after, or is assigned: nothing to the clock,
    /// and what `woken` refuses a stream for (fm3 log 103)
    Given(String),
}

/// what a phrase calls
enum Callee {
    /// one of the compiler's own words, or a variable: nothing
    None,
    End(String),
    Unknown,
    /// a task run now, with the arguments
    Run(Vec<usize>, Vec<Expr>),
    /// the store's functions, with the arguments
    Fns(Vec<usize>, Vec<Expr>),
}

/// the names a body has bound, and whether each is plainly one value
type Scope = HashMap<String, bool>;

fn gcd(a: i64, b: i64) -> i64 {
    if b == 0 { a } else { gcd(b, a % b) }
}

/// is a declared name plainly one value? A `$` and a string are streams
fn single(seq: bool, ty: &str) -> bool {
    !seq && ty != "string"
}

impl<'a> Beat<'a> {
    fn new(l: &'a Lowerer, store: &'a Store) -> Beat<'a> {
        let mut defs = Vec::new();
        let mut add = |fd: &'a FnDecl, at: usize, edge: bool, user: bool| {
            let key = mangle(&fd.name);
            let arity = fd.params().count();
            let sym = fd.name.iter().any(|p| matches!(p, NamePart::Sym(_)));
            let task = fd.task || l.funcs.iter().any(|g| g.key == key && g.parts == fd.name && g.params.len() == arity && g.task);
            defs.push(BeatDef { fd, key, arity, at, plain: !edge && !sym && !task && fd.platform.is_empty(), sym, user, task });
        };
        for (at, f) in store.features.iter().enumerate() {
            for d in &f.code.decls {
                if let Decl::Fn(fd) = d {
                    add(fd, at, false, f.name != "platform");
                }
            }
        }
        for (fd, feature, _) in &l.edges {
            add(fd, l.features.iter().position(|f| f == feature).unwrap_or(0), true, false);
        }
        let mut by_call: HashMap<(String, usize), Vec<usize>> = HashMap::new();
        for (i, d) in defs.iter().enumerate() {
            by_call.entry((d.key.clone(), d.arity)).or_default().push(i);
        }
        let n = defs.len();
        let user_syms = defs.iter().any(|d| d.sym && d.user);
        Beat { l, defs, by_call, evs: Vec::new(), moves: vec![false; n], nodes_move: false, sym_moves: false, user_syms, entry: vec![0; n], exit: vec![0; n], looped: vec![false; n], loops: Vec::new(), record: false, need: HashMap::new(), hoist: HashMap::new() }
    }

    /// a definition's parameters and results, bound
    fn scope_of(fd: &FnDecl) -> Scope {
        fd.results.iter().chain(fd.params()).map(|p| (p.name.clone(), single(p.seq, &p.ty))).collect()
    }

    /// the edges out of a stream with no storage, as definitions
    fn edges_of(&self, n: &str) -> Vec<usize> {
        let mut out = Vec::new();
        for (edge, _) in self.l.bare_edges.get(n).map(|v| v.as_slice()).unwrap_or_default() {
            out.extend(self.by_call.get(&(edge.clone(), 1)).into_iter().flatten().copied());
        }
        out
    }

    /// the pushes on the beat and the loops that align before they
    /// begin, by the statement's address
    fn solve(mut self, store: &Store) -> (std::collections::HashSet<usize>, HashMap<usize, i64>) {
        let n = self.defs.len();
        let mut evs = Vec::new();
        for d in 0..n {
            let mut out = Vec::new();
            let fd = self.defs[d].fd;
            self.events(&fd.body, Some(d), &Self::scope_of(fd), &mut out);
            evs.push(out);
        }
        self.evs = evs;
        // what can move the clock: the least set that holds
        loop {
            let mut changed = false;
            for d in 0..n {
                if self.moves[d] {
                    continue;
                }
                let def = &self.defs[d];
                if !def.fd.platform.is_empty() || (def.task && self.l.any_rated_wiring) || self.evs[d].iter().any(|e| self.moved_by(e)) {
                    self.moves[d] = true;
                    changed = true;
                }
            }
            let sym = (0..n).any(|d| self.defs[d].sym && self.moves[d]);
            let nodes = self.l.any_rated_wiring || self.l.nodes.iter().any(|k| k.hz > 0) || (0..n).any(|d| self.defs[d].task && self.moves[d]);
            if sym != self.sym_moves || nodes != self.nodes_move {
                self.sym_moves = sym;
                self.nodes_move = nodes;
                changed = true;
            }
            if !changed {
                break;
            }
        }
        if self.sym_moves {
            return (std::collections::HashSet::new(), HashMap::new());
        }
        // a definition that can reach itself leaves nothing known
        let syms: Vec<usize> = (0..n).filter(|&d| self.defs[d].sym).collect();
        let tasks: Vec<usize> = (0..n).filter(|&d| self.defs[d].task).collect();
        let next = |d: usize| -> Vec<usize> {
            let mut v = Vec::new();
            for ev in &self.evs[d] {
                match ev {
                    Ev::Push(s) => {
                        v.extend(self.edges_of(s));
                        v.extend(&syms);
                        if self.l.node_inputs.contains(s) {
                            v.extend(&tasks);
                        }
                    }
                    Ev::Op => v.extend(&syms),
                    Ev::End(_) => v.extend(&tasks),
                    Ev::Unknown | Ev::Given(_) => {}
                    Ev::Call(ts) => v.extend(ts),
                }
            }
            v
        };
        let mut looped = vec![false; n];
        for d in 0..n {
            let mut seen = vec![false; n];
            let mut work = next(d);
            while let Some(t) = work.pop() {
                if t == d {
                    looped[d] = true;
                    break;
                }
                if !std::mem::replace(&mut seen[t], true) {
                    work.extend(next(t));
                }
            }
        }
        self.looped = looped;
        // a case starts at 0 unless the start can move the clock: a node
        // that can, or an initial value that calls something that can,
        // the reset zeroing the clock before the initial values
        let mut inits = Vec::new();
        for f in &store.features {
            for d in &f.code.decls {
                if let Decl::Var(v) = d {
                    self.events_init(v, None, &Scope::new(), &mut inits);
                }
            }
        }
        let start = if self.nodes_move || inits.iter().any(|e| self.moved_by(e)) { 1 } else { 0 };
        self.entry = vec![start; n];
        // what is not a plain function calls at nothing known
        for d in 0..n {
            if self.defs[d].plain {
                continue;
            }
            for ev in &self.evs[d] {
                if let Ev::Call(ts) = ev {
                    for &t in ts {
                        self.entry[t] = 1;
                    }
                }
            }
        }
        // round the plain functions until no fact changes, then once
        // more to write the choices down
        loop {
            let before = (self.entry.clone(), self.exit.clone());
            self.round();
            if (&self.entry, &self.exit) == (&before.0, &before.1) {
                break;
            }
        }
        self.record = true;
        self.round();
        let on_beat = self.need.iter().filter(|(_, need)| !**need).map(|(s, _)| *s).collect();
        let before = self.hoist.iter().filter_map(|(s, hz)| hz.map(|hz| (*s, hz))).collect();
        (on_beat, before)
    }

    /// Which streams only plain functions wake, and whether the
    /// scheduler's guard can ever be found busy (fm3 log 103). A stream
    /// a node reads is woken by its pushers when every push into it and
    /// every `end` of it stands in a plain function that nothing the
    /// scheduler runs and nothing the reset evaluates can reach: its
    /// trigger is then never met while a node runs, and every arrival
    /// is followed by one, so the node is due exactly when a statement
    /// has just pushed or first ended, and need not ask. Refused, each
    /// because items could then arrive with no trigger after them, or a
    /// trigger be met elsewhere: a stream with anything on its
    /// declaration, or the platform's; one handed to a call or a task
    /// run, assigned, or pushed a task call's items (a name a body has
    /// bound is that body's own); and every stream of a store with a
    /// `platform` body of its own, which is IR this pass does not read.
    /// The guard is dead where no plain body reached from a task, a
    /// sink or an edge's function pushes into or ends a node's input
    fn woken(mut self, store: &Store) -> (Names, bool) {
        let n = self.defs.len();
        let mut evs = Vec::new();
        for d in 0..n {
            let mut out = Vec::new();
            let fd = self.defs[d].fd;
            self.events(&fd.body, Some(d), &Self::scope_of(fd), &mut out);
            evs.push(out);
        }
        self.evs = evs;
        // what the reset evaluates: every initial value but a wiring,
        // which is a node and runs at the start
        let mut inits = Vec::new();
        for f in &store.features {
            for d in &f.code.decls {
                let Decl::Var(v) = d else { continue };
                let wired = |e: &Expr| matches!(self.l.task_call(e, None, &f.code.file), Ok(Some(_)));
                match &v.init {
                    Some(Init::Value(e)) if !wired(e) => self.events_expr(e, None, &Scope::new(), &mut inits),
                    Some(Init::Construct(args)) => args.iter().for_each(|a| self.events_expr(&a.value, None, &Scope::new(), &mut inits)),
                    Some(Init::Pushes { items, cond }) => {
                        inits.push(Ev::Op);
                        items.iter().chain(cond.iter()).filter(|e| !wired(e)).for_each(|e| self.events_expr(e, None, &Scope::new(), &mut inits));
                    }
                    _ => {}
                }
            }
        }
        // a `platform` body of the store's own is not read: it may do anything
        let mut all = self.defs.iter().any(|d| d.user && !d.fd.platform.is_empty());
        // what a node's run or the reset can reach
        let syms: Vec<usize> = (0..n).filter(|&d| self.defs[d].sym).collect();
        let mut inside = vec![false; n];
        let mut work: Vec<usize> = (0..n).filter(|&d| self.defs[d].task).collect();
        let step = |ev: &Ev, work: &mut Vec<usize>, all: &mut bool| match ev {
            Ev::Push(s) => {
                work.extend(self.edges_of(s));
                work.extend(&syms);
            }
            Ev::Op => work.extend(&syms),
            Ev::Call(ts) => work.extend(ts),
            Ev::Unknown => *all = true,
            Ev::End(_) | Ev::Given(_) => {}
        };
        for ev in &inits {
            step(ev, &mut work, &mut all);
        }
        while let Some(d) = work.pop() {
            if std::mem::replace(&mut inside[d], true) {
                continue;
            }
            for ev in &self.evs[d] {
                step(ev, &mut work, &mut all);
            }
        }
        if all {
            return (Names::new(), true);
        }
        let inputs = &self.l.node_inputs;
        let touches = |ev: &Ev, s: &str| matches!(ev, Ev::Push(x) | Ev::End(x) if x == s);
        // a task's own body is lowered with no trigger in it
        let guard = (0..n).any(|d| inside[d] && !self.defs[d].task && self.evs[d].iter().any(|e| inputs.iter().any(|s| touches(e, s))));
        let mut out = Names::new();
        for s in inputs {
            let declared = store.features.iter().any(|f| f.name != "platform" && f.code.decls.iter().any(|d| matches!(d, Decl::Var(v) if &v.name == s && v.seq && v.init.is_none())));
            let mut ok = declared && !inits.iter().any(|e| touches(e, s) || matches!(e, Ev::Given(x) if x == s));
            for d in 0..n {
                let def = &self.defs[d];
                for ev in &self.evs[d] {
                    if touches(ev, s) {
                        ok &= def.plain && !inside[d];
                    }
                    if matches!(ev, Ev::Given(x) if x == s) {
                        ok = false;
                    }
                }
            }
            if ok {
                out.insert(s.clone());
            }
        }
        (out, guard)
    }

    fn round(&mut self) {
        for d in 0..self.defs.len() {
            if !self.defs[d].plain {
                continue;
            }
            let fd = self.defs[d].fd;
            // a function with results ends where its last result is
            // assigned, so what it leaves is the weakest fact after any
            // of its statements
            let mut leaves = 0;
            let end = self.flow_block(&fd.body, self.entry[d], d, &Self::scope_of(fd), &mut leaves);
            let out = if self.looped[d] { 1 } else if fd.results.is_empty() { end } else { gcd(end, leaves) };
            self.exit[d] = gcd(self.exit[d], out);
        }
    }

    /// can this move the clock?
    fn moved_by(&self, ev: &Ev) -> bool {
        match ev {
            Ev::Push(n) => self.sym_moves || self.l.rates.contains_key(n) || self.edges_of(n).iter().any(|&t| self.moves[t]) || (self.l.node_inputs.contains(n) && self.nodes_move),
            Ev::Op => self.sym_moves,
            Ev::End(n) => self.l.node_inputs.contains(n) && self.nodes_move,
            Ev::Unknown => true,
            Ev::Call(ts) => ts.iter().any(|&t| self.moves[t]),
            Ev::Given(_) => false,
        }
    }

    /// what a phrase calls: the store's functions, any the name and the
    /// argument count may reach; else `end x$`, after which the
    /// scheduler may run; else one of the compiler's own words, which
    /// move nothing; anything else is not known
    fn callee(&self, e: &Expr, d: Option<usize>, scope: &Scope) -> Callee {
        let is_var = |w: &str| scope.contains_key(w) || self.l.fvar(w).is_some();
        let parts = match &e.kind {
            ExprKind::Phrase(parts) => parts,
            // `existing f(...)`: a definition in a feature composed before this one
            ExprKind::Existing(parts) => {
                let Some(d) = d else { return Callee::Unknown };
                let def = &self.defs[d];
                let mut args = Vec::new();
                for p in parts {
                    match p {
                        Part::Args(list) => args.extend(list.iter().map(|a| a.value.clone())),
                        Part::Value(x) => args.push(x.clone()),
                        Part::Word(w) if is_var(w) => args.push(Expr { kind: ExprKind::Name(w.clone()), line: e.line }),
                        Part::Word(_) => {}
                    }
                }
                let below = self.by_call.get(&(def.key.clone(), def.arity)).into_iter().flatten().copied().filter(|&t| self.defs[t].at < def.at).collect();
                return Callee::Fns(below, args);
            }
            _ => return Callee::None,
        };
        let seq = |p: &Part| matches!(p, Part::Value(Expr { kind: ExprKind::Seq(_), .. }));
        let own = match parts.as_slice() {
            [Part::Word(w)] => is_var(w) || w == "enabled" || self.l.enum_case(w).is_some(),
            [Part::Word(w), Part::Args(_)] if self.l.types.contains_key(w) || builtin_type(w).is_some() => true,
            [Part::Word(w), Part::Value(Expr { kind: ExprKind::List(_), .. })] => is_var(w),
            [Part::Word(t), Part::Word(of), x] if t == "time" && of == "of" => seq(x),
            [Part::Word(w), x] if seq(x) => matches!(w.as_str(), "count" | "latest" | "frame" | "ended" | "position" | "end"),
            [Part::Word(w), x, Part::Word(k), _] if seq(x) => (w == "peek" && k == "at") || (w == "advance" && k == "by"),
            [x, Part::Word(w), _] if seq(x) => w == "behind" || w == "at",
            [x, Part::Word(f), _, Part::Word(t), _] if seq(x) => f == "from" && t == "to",
            [Part::Word(w), Part::Word(u)] => is_var(w) && matches!(u.as_str(), "s" | "ms" | "us" | "ns"),
            [Part::Word(w), _] => w == "count",
            _ => false,
        };
        let reached = |cands: &[&FnInfo], n: usize| -> Vec<usize> {
            let mut ts = Vec::new();
            for c in cands {
                for &t in self.by_call.get(&(c.key.clone(), n)).into_iter().flatten() {
                    if !ts.contains(&t) {
                        ts.push(t);
                    }
                }
            }
            ts
        };
        if let Ok((cands, args)) = find_methods(&self.l.funcs, parts, &is_var, "", e.line) {
            let ts = reached(&cands, args.len());
            // a function of the store's spelled as one of the compiler's
            // words: which of the two the lowering takes is its to say
            return if ts.is_empty() || own {
                Callee::Unknown
            } else if cands.iter().any(|c| c.task) {
                Callee::Run(ts, args)
            } else {
                Callee::Fns(ts, args)
            };
        }
        // a task run at a rate sleeps
        if let [rest @ .., Part::Word(at), Part::Args(a)] = parts.as_slice() {
            if at == "at" && a.len() == 1 && matches!(a[0].value.kind, ExprKind::Unit(..)) && find_methods(&self.l.funcs, rest, &is_var, "", e.line).is_ok_and(|(c, _)| c.iter().any(|c| c.task)) {
                return Callee::Unknown;
            }
        }
        if let [Part::Word(w), Part::Value(Expr { kind: ExprKind::Seq(n), .. })] = parts.as_slice() {
            if w == "end" {
                return Callee::End(n.clone());
            }
        }
        if own { Callee::None } else { Callee::Unknown }
    }

    /// what a block may do to the clock, in no order
    fn events(&self, stmts: &[Stmt], d: Option<usize>, scope: &Scope, out: &mut Vec<Ev>) {
        let mut scope = scope.clone();
        for s in stmts {
            match s {
                Stmt::Var(v) => {
                    self.events_init(v, d, &scope, out);
                    scope.insert(v.name.clone(), single(v.seq, &v.ty));
                }
                Stmt::Multi { vars, value, .. } => {
                    self.events_expr(value, d, &scope, out);
                    for p in vars {
                        scope.insert(p.name.clone(), single(p.seq, &p.ty));
                    }
                }
                Stmt::Assign { targets, value, .. } => {
                    targets.iter().filter(|t| !scope.contains_key(&t.name)).for_each(|t| out.push(Ev::Given(t.name.clone())));
                    self.events_expr(value, d, &scope, out);
                }
                Stmt::Expr { expr: value, .. } | Stmt::Check { cond: value, .. } => self.events_expr(value, d, &scope, out),
                Stmt::If { cond, then, els, .. } => {
                    self.events_expr(cond, d, &scope, out);
                    self.events(then, d, &scope, out);
                    if let Some(e) = els {
                        self.events(e, d, &scope, out);
                    }
                }
                Stmt::Loop { vars, cond, body, into, .. } => {
                    let mut inner = scope.clone();
                    for v in vars {
                        self.events_init(v, d, &inner, out);
                        inner.insert(v.name.clone(), single(v.seq, &v.ty));
                    }
                    if let Some(c) = cond {
                        self.events_expr(c, d, &inner, out);
                    }
                    self.events(body, d, &inner, out);
                    match into {
                        Some(LoopInto::Declare(ps)) => {
                            for p in ps {
                                scope.insert(p.name.clone(), single(p.seq, &p.ty));
                            }
                        }
                        Some(LoopInto::Assign(ts)) => ts.iter().filter(|t| !scope.contains_key(&t.name)).for_each(|t| out.push(Ev::Given(t.name.clone()))),
                        None => {}
                    }
                }
                Stmt::For { var, seq, body, .. } => {
                    self.events_expr(seq, d, &scope, out);
                    let mut inner = scope.clone();
                    inner.insert(var.clone(), false);
                    self.events(body, d, &inner, out);
                }
                Stmt::Continue { values, .. } => values.iter().for_each(|e| self.events_expr(e, d, &scope, out)),
                Stmt::Break { .. } => {}
                Stmt::Push { target, items, cond, .. } => {
                    out.push(match &target.kind {
                        ExprKind::Seq(n) => Ev::Push(n.clone()),
                        _ => Ev::Unknown,
                    });
                    // an item that is a task call, or a phrase nothing
                    // can name, pushes inside itself
                    if let ExprKind::Seq(n) = &target.kind {
                        if !scope.contains_key(n) && items.iter().any(|e| matches!(e.kind, ExprKind::Phrase(_)) && matches!(self.callee(e, d, &scope), Callee::Run(..) | Callee::Unknown)) {
                            out.push(Ev::Given(n.clone()));
                        }
                    }
                    items.iter().chain(cond.iter()).for_each(|e| self.events_expr(e, d, &scope, out));
                }
            }
        }
    }

    fn events_init(&self, v: &super::syntax::VarDecl, d: Option<usize>, scope: &Scope, out: &mut Vec<Ev>) {
        match &v.init {
            Some(Init::Value(e)) => self.events_expr(e, d, scope, out),
            Some(Init::Construct(args)) => args.iter().for_each(|a| self.events_expr(&a.value, d, scope, out)),
            Some(Init::Pushes { items, cond }) => {
                out.push(Ev::Op);
                items.iter().chain(cond.iter()).for_each(|e| self.events_expr(e, d, scope, out));
            }
            None => {}
        }
    }

    fn events_expr(&self, e: &Expr, d: Option<usize>, scope: &Scope, out: &mut Vec<Ev>) {
        match &e.kind {
            ExprKind::Unit(x, _) | ExprKind::Field(x, _) => self.events_expr(x, d, scope, out),
            ExprKind::Neg(x) => {
                out.push(Ev::Op);
                self.events_expr(x, d, scope, out);
            }
            ExprKind::List(items) => items.iter().for_each(|x| self.events_expr(x, d, scope, out)),
            ExprKind::Range { from, to, .. } => {
                self.events_expr(from, d, scope, out);
                self.events_expr(to, d, scope, out);
            }
            ExprKind::Bin(_, l, r) | ExprKind::Index(l, r) => {
                out.push(Ev::Op);
                self.events_expr(l, d, scope, out);
                self.events_expr(r, d, scope, out);
            }
            ExprKind::IfElse(c, t, f) => {
                self.events_expr(c, d, scope, out);
                self.events_expr(t, d, scope, out);
                self.events_expr(f, d, scope, out);
            }
            ExprKind::Phrase(parts) | ExprKind::Existing(parts) => {
                for p in parts {
                    match p {
                        Part::Args(list) => list.iter().for_each(|a| self.events_expr(&a.value, d, scope, out)),
                        Part::Value(x) => self.events_expr(x, d, scope, out),
                        Part::Word(_) => {}
                    }
                }
                // a stream among the arguments is handed to what is
                // called; where the phrase cannot be named, any stream
                // it mentions is
                let given = |x: &Expr, out: &mut Vec<Ev>| {
                    if let ExprKind::Seq(n) | ExprKind::Name(n) = &x.kind {
                        if !scope.contains_key(n) {
                            out.push(Ev::Given(n.clone()));
                        }
                    }
                };
                match self.callee(e, d, scope) {
                    Callee::None => {}
                    Callee::End(n) => out.push(Ev::End(n)),
                    Callee::Unknown => {
                        out.push(Ev::Unknown);
                        for p in parts {
                            match p {
                                Part::Args(list) => list.iter().for_each(|a| given(&a.value, out)),
                                Part::Value(x) => given(x, out),
                                Part::Word(w) if !scope.contains_key(w) => out.push(Ev::Given(w.clone())),
                                Part::Word(_) => {}
                            }
                        }
                    }
                    Callee::Run(ts, args) | Callee::Fns(ts, args) => {
                        args.iter().for_each(|a| given(a, out));
                        out.push(Ev::Call(ts));
                    }
                }
            }
            _ => {}
        }
    }

    /// the fact after a block that began at `g`; `leaves` takes the
    /// weakest fact after any statement
    fn flow_block(&mut self, stmts: &[Stmt], mut g: i64, d: usize, scope: &Scope, leaves: &mut i64) -> i64 {
        let mut scope = scope.clone();
        for s in stmts {
            g = self.flow_stmt(s, g, d, &mut scope, leaves);
            *leaves = gcd(*leaves, g);
        }
        g
    }

    fn flow_stmt(&mut self, s: &Stmt, mut g: i64, d: usize, scope: &mut Scope, leaves: &mut i64) -> i64 {
        match s {
            Stmt::Var(v) => {
                g = self.flow_init(v, g, d, scope);
                scope.insert(v.name.clone(), single(v.seq, &v.ty));
                g
            }
            Stmt::Multi { vars, value, .. } => {
                g = self.flow_expr(value, g, d, scope);
                for p in vars {
                    scope.insert(p.name.clone(), single(p.seq, &p.ty));
                }
                g
            }
            Stmt::Assign { value, .. } | Stmt::Expr { expr: value, .. } | Stmt::Check { cond: value, .. } => self.flow_expr(value, g, d, scope),
            Stmt::If { cond, then, els, .. } => {
                g = self.flow_expr(cond, g, d, scope);
                let a = self.flow_block(then, g, d, scope, leaves);
                let b = match els {
                    Some(e) => self.flow_block(e, g, d, scope, leaves),
                    None => g,
                };
                gcd(a, b)
            }
            Stmt::Loop { vars, cond, body, into, .. } => {
                let mut inner = scope.clone();
                for v in vars {
                    g = self.flow_init(v, g, d, &inner);
                    inner.insert(v.name.clone(), single(v.seq, &v.ty));
                }
                let cond = cond.as_ref();
                let (head, mut out) = self.flow_loop(cond, body, g, d, &inner, leaves, false);
                // the body's first statement pushes into a stream with a
                // rate and would align at the head: if with the head on
                // that beat every way round leaves it there, the
                // alignment is made once before the loop, where the loop
                // has a first pass. The `while` is then asked once more,
                // of the initial values, so it must be safe to ask twice
                let mut before = None;
                let first = match body.first() {
                    Some(Stmt::Push { target: Expr { kind: ExprKind::Seq(n), .. }, existing: false, .. }) if !inner.contains_key(n) => self.l.rates.get(n).copied().filter(|&hz| super::store::period(hz) > 1),
                    _ => None,
                };
                if let Some(hz) = first {
                    let q = super::store::period(hz);
                    if head % q != 0 && !self.user_syms && cond.map_or(true, |c| self.twice(c, &inner)) {
                        let (head, tried) = self.flow_loop(cond, body, q, d, &inner, leaves, false);
                        if head % q == 0 {
                            before = Some(hz);
                            out = if cond.is_some() { gcd(g, tried) } else { tried };
                        } else {
                            // not proved: the facts and the choices are the first walk's
                            self.flow_loop(cond, body, g, d, &inner, leaves, false);
                        }
                    }
                }
                if self.record {
                    self.hoist.insert(s as *const Stmt as usize, before);
                }
                if let Some(LoopInto::Declare(ps)) = into {
                    for p in ps {
                        scope.insert(p.name.clone(), single(p.seq, &p.ty));
                    }
                }
                out
            }
            Stmt::For { var, seq, body, .. } => {
                g = self.flow_expr(seq, g, d, scope);
                let mut inner = scope.clone();
                inner.insert(var.clone(), false);
                self.flow_loop(None, body, g, d, &inner, leaves, true).1
            }
            Stmt::Break { .. } => {
                if let Some(l) = self.loops.last_mut() {
                    l.0 = Some(l.0.map_or(g, |b| gcd(b, g)));
                }
                g
            }
            Stmt::Continue { values, .. } => {
                for e in values {
                    g = self.flow_expr(e, g, d, scope);
                }
                if let Some(l) = self.loops.last_mut() {
                    l.1 = Some(l.1.map_or(g, |b| gcd(b, g)));
                }
                g
            }
            Stmt::Push { target, items, cond, existing, .. } => {
                let ExprKind::Seq(n) = &target.kind else { return 1 };
                let es: Vec<&Expr> = items.iter().chain(cond.iter()).collect();
                // `existing o$ << x` is a method's, and no method moves the clock
                if *existing {
                    return self.flow_again(&es, g, d, scope);
                }
                let period = if scope.contains_key(n) { None } else { self.l.rates.get(n).map(|&hz| super::store::period(hz)) };
                // at a rate the statement's first item lands on the
                // beat: where the period divides the fact it is there
                // already, and otherwise the alignment puts it there
                let mut at = g;
                if let Some(q) = period.filter(|&q| q > 1) {
                    let on = g % q == 0;
                    if self.record {
                        self.need.insert(s as *const Stmt as usize, !on);
                    }
                    if !on {
                        at = q;
                    }
                }
                // each item is then a push and a step, so every later
                // item is evaluated at a multiple of both
                let lo = match period {
                    None => at,
                    Some(q) if q > 1 => gcd(at, q),
                    Some(_) => 1,
                };
                // ... provided what takes the items cannot move the
                // clock: the edges out of a stream with no storage, the
                // nodes the scheduler runs after a push into one they read
                let taken = !self.edges_of(n).iter().any(|&t| self.moves[t]) && !(self.l.node_inputs.contains(n) && self.nodes_move);
                let after = self.flow_again(&es, if taken { lo } else { 1 }, d, scope);
                if taken { after } else { 1 }
            }
        }
    }

    fn flow_init(&mut self, v: &super::syntax::VarDecl, mut g: i64, d: usize, scope: &Scope) -> i64 {
        match &v.init {
            Some(Init::Value(e)) => self.flow_expr(e, g, d, scope),
            Some(Init::Construct(args)) => {
                for a in args {
                    g = self.flow_expr(&a.value, g, d, scope);
                }
                g
            }
            Some(Init::Pushes { items, cond }) => self.flow_again(&items.iter().chain(cond.iter()).collect::<Vec<_>>(), g, d, scope),
            None => g,
        }
    }

    /// expressions that may be evaluated more than once, each where the
    /// last left the clock: the fact they began at if none changes it,
    /// and nothing known otherwise
    fn flow_again(&mut self, es: &[&Expr], g: i64, d: usize, scope: &Scope) -> i64 {
        let mut same = true;
        for e in es {
            same &= self.flow_expr(e, g, d, scope) == g;
        }
        if same {
            return g;
        }
        for e in es {
            self.flow_expr(e, 1, d, scope);
        }
        1
    }

    /// a loop from `g`: the fact at its head, the weakest of the entry
    /// and every way back, and the fact after it. A `for` leaves from
    /// its head, a `while` from its test
    fn flow_loop(&mut self, cond: Option<&Expr>, body: &[Stmt], g: i64, d: usize, scope: &Scope, leaves: &mut i64, each: bool) -> (i64, i64) {
        let mut head = g;
        loop {
            let tested = match cond {
                Some(c) => self.flow_expr(c, head, d, scope),
                None => head,
            };
            self.loops.push((None, None));
            let end = self.flow_block(body, tested, d, scope, leaves);
            let (breaks, backs) = self.loops.pop().unwrap();
            let next = gcd(head, backs.map_or(end, |b| gcd(b, end)));
            if next == head {
                let left = if each { head } else { tested };
                let out = match (cond.is_some() || each, breaks) {
                    (true, Some(b)) => gcd(left, b),
                    (true, None) => left,
                    (false, Some(b)) => b,
                    (false, None) => tested,
                };
                return (head, out);
            }
            head = next;
        }
    }

    fn flow_expr(&mut self, e: &Expr, mut g: i64, d: usize, scope: &Scope) -> i64 {
        match &e.kind {
            ExprKind::Unit(x, _) | ExprKind::Neg(x) | ExprKind::Field(x, _) => self.flow_expr(x, g, d, scope),
            ExprKind::List(items) => {
                for x in items {
                    g = self.flow_expr(x, g, d, scope);
                }
                g
            }
            ExprKind::Range { from: l, to: r, .. } | ExprKind::Bin(_, l, r) | ExprKind::Index(l, r) => {
                g = self.flow_expr(l, g, d, scope);
                self.flow_expr(r, g, d, scope)
            }
            ExprKind::IfElse(c, t, f) => {
                g = self.flow_expr(c, g, d, scope);
                gcd(self.flow_expr(t, g, d, scope), self.flow_expr(f, g, d, scope))
            }
            ExprKind::Phrase(parts) | ExprKind::Existing(parts) => {
                for p in parts {
                    match p {
                        Part::Args(list) => {
                            for a in list {
                                g = self.flow_expr(&a.value, g, d, scope);
                            }
                        }
                        Part::Value(x) => g = self.flow_expr(x, g, d, scope),
                        Part::Word(_) => {}
                    }
                }
                match self.callee(e, Some(d), scope) {
                    Callee::None => g,
                    Callee::End(n) => if self.l.node_inputs.contains(&n) && self.nodes_move { 1 } else { g },
                    Callee::Unknown | Callee::Run(..) => 1,
                    Callee::Fns(ts, args) => self.flow_call(&ts, &args, g, matches!(e.kind, ExprKind::Existing(_)), scope),
                }
            }
            _ => g,
        }
    }

    /// a call at `g` that may enter any of `ts`: each takes the fact at
    /// its entry, and the call leaves the weakest of what they leave —
    /// and of `g` itself where it goes through a chain's links, since
    /// every feature in the chain may be off
    fn flow_call(&mut self, ts: &[usize], args: &[Expr], g: i64, link: bool, scope: &Scope) -> i64 {
        if ts.is_empty() {
            return g;
        }
        let mut after = if ts.len() == 1 && !link { None } else { Some(g) };
        for &t in ts {
            let x = if !self.moves[t] {
                g
            } else if self.defs[t].plain && !self.looped[t] {
                self.exit[t]
            } else {
                1
            };
            after = Some(after.map_or(x, |a| gcd(a, x)));
        }
        let after = after.unwrap();
        // an argument that is not plainly one value may be a stream the
        // function is applied to item by item: it is then entered again
        // where its last run left the clock
        let name = |n: &str| scope.get(n).copied().unwrap_or_else(|| self.l.fvar(n).is_some_and(|f| !matches!(f.ty, Ty::Stream(_))));
        let one = args.iter().all(|a| match &a.kind {
            ExprKind::Int(_) | ExprKind::Float(_) | ExprKind::Bool(_) => true,
            ExprKind::Name(n) => name(n),
            ExprKind::Phrase(parts) => matches!(parts.as_slice(), [Part::Word(w)] if name(w)),
            _ => false,
        });
        let at = if one { g } else { gcd(g, after) };
        for &t in ts {
            if self.defs[t].plain {
                self.entry[t] = gcd(self.entry[t], at);
            }
        }
        after
    }

    /// may a loop's `while` be asked twice? Literals, names, operators
    /// and the words that read a stream without moving it
    fn twice(&self, e: &Expr, scope: &Scope) -> bool {
        match &e.kind {
            ExprKind::Int(_) | ExprKind::Float(_) | ExprKind::Bool(_) | ExprKind::Str(_) | ExprKind::Name(_) | ExprKind::Seq(_) => true,
            ExprKind::Unit(x, _) | ExprKind::Neg(x) | ExprKind::Field(x, _) => self.twice(x, scope),
            ExprKind::Bin(_, l, r) | ExprKind::Index(l, r) => self.twice(l, scope) && self.twice(r, scope),
            ExprKind::IfElse(c, t, f) => self.twice(c, scope) && self.twice(t, scope) && self.twice(f, scope),
            ExprKind::Phrase(parts) => {
                let is_var = |w: &str| scope.contains_key(w) || self.l.fvar(w).is_some();
                if find_methods(&self.l.funcs, parts, &is_var, "", e.line).is_ok() {
                    return false;
                }
                let seq = |p: &Part| matches!(p, Part::Value(Expr { kind: ExprKind::Seq(_), .. }));
                match parts.as_slice() {
                    [Part::Word(w)] => is_var(w),
                    [Part::Word(w), x] if seq(x) => matches!(w.as_str(), "count" | "ended" | "position" | "latest"),
                    [Part::Word(w), x, Part::Word(at), Part::Value(i)] if seq(x) && w == "peek" && at == "at" => self.twice(i, scope),
                    [Part::Word(w), x, Part::Word(at), Part::Args(a)] if seq(x) && w == "peek" && at == "at" => a.iter().all(|a| self.twice(&a.value, scope)),
                    _ => false,
                }
            }
            _ => false,
        }
    }
}

impl Lowerer {
    /// the scheduler's order (log 78): the nodes in a stable topological
    /// order of the graph node → stream it may push into → node reading
    /// it (declaration order among the unrelated), and the stream a cycle
    /// runs through if there is one
    fn schedule(&self, store: &Store, nodes: &[Node]) -> Option<(Vec<usize>, Vec<Names>, Option<String>)> {
        let mut bodies: HashMap<String, Vec<(Vec<String>, &[Stmt])>> = HashMap::new();
        fn add<'b>(fd: &'b FnDecl, bodies: &mut HashMap<String, Vec<(Vec<String>, &'b [Stmt])>>) {
            let mut params: Vec<String> = fd.results.iter().map(|p| p.name.clone()).collect();
            params.extend(fd.params().map(|p| p.name.clone()));
            bodies.entry(mangle(&fd.name)).or_default().push((params, fd.body.as_slice()));
        }
        for f in &store.features {
            for d in &f.code.decls {
                if let Decl::Fn(fd) = d {
                    add(fd, &mut bodies);
                }
            }
        }
        for (fd, _, _) in &self.edges {
            add(fd, &mut bodies);
        }
        let mut walker = Pushes { l: self, bodies, known: HashMap::new(), active: Names::new(), file: store.product_file.clone(), depth: 0, handed: None };
        let n = nodes.len();
        let mut reads: Vec<Names> = Vec::new();
        let mut writes: Vec<Names> = Vec::new();
        for node in nodes {
            let mut r = Names::new();
            for (a, (_, pty)) in node.args.iter().zip(&node.info.params) {
                if let (ExprKind::Seq(s), Ty::Stream(_)) = (&a.kind, pty) {
                    r.insert(s.clone());
                }
            }
            let (theirs, pushed) = walker.of_key(&node.info.key);
            let mut w = theirs;
            if let Some(o) = &node.out {
                w.insert(o.clone());
            }
            for (a, (pname, _)) in node.args.iter().zip(&node.info.params) {
                if let (ExprKind::Seq(s), true) = (&a.kind, pushed.contains(pname)) {
                    w.insert(s.clone());
                }
            }
            // a push into a stream with no storage is its edges' call
            // (question 50): what they may push into is pushed too
            loop {
                let mut more = Names::new();
                for s in &w {
                    for (edge, _) in self.bare_edges.get(s).map(|v| v.as_slice()).unwrap_or_default() {
                        more.extend(walker.of_key(edge).0);
                    }
                }
                let before = w.len();
                w.extend(more);
                if w.len() == before {
                    break;
                }
            }
            reads.push(r);
            writes.push(w);
        }
        // edges k → j where k may push into what j reads
        let mut succs: Vec<Vec<usize>> = vec![Vec::new(); n];
        let mut indeg = vec![0usize; n];
        for k in 0..n {
            for j in 0..n {
                if let Some(s) = writes[k].iter().find(|s| reads[j].contains(*s)).cloned() {
                    if k == j {
                        return Some((Vec::new(), writes, Some(s)));
                    }
                    succs[k].push(j);
                    indeg[j] += 1;
                }
            }
        }
        // Kahn's order, the lowest declaration index first
        let mut order = Vec::new();
        let mut ready: std::collections::BTreeSet<usize> = (0..n).filter(|&k| indeg[k] == 0).collect();
        while let Some(&k) = ready.iter().next() {
            ready.remove(&k);
            order.push(k);
            for &j in &succs[k] {
                indeg[j] -= 1;
                if indeg[j] == 0 {
                    ready.insert(j);
                }
            }
        }
        if order.len() < n {
            let k = (0..n).find(|k| !order.contains(k)).unwrap();
            let through = writes[k].iter().find(|s| (0..n).any(|j| !order.contains(&j) && reads[j].contains(*s))).cloned().unwrap_or_default();
            return Some((Vec::new(), writes, Some(through)));
        }
        Some((order, writes, None))
    }

    /// the nodes a push into `s$` reaches, in the schedule's order: those
    /// reading it, then those reading what they may push into
    fn reached(&self, s: &str, nodes: &[Node], order: &[usize], writes: &[Names]) -> Vec<usize> {
        let reads = |k: usize| -> Names {
            nodes[k].args.iter().zip(&nodes[k].info.params).filter_map(|(a, (_, t))| match (&a.kind, t) { (ExprKind::Seq(n), Ty::Stream(_)) => Some(n.clone()), _ => None }).collect()
        };
        let mut live: Names = Names::new();
        live.insert(s.to_string());
        let mut out = Vec::new();
        for &k in order {
            if reads(k).iter().any(|r| live.contains(r)) {
                out.push(k);
                live.extend(writes[k].iter().cloned());
            }
        }
        out
    }
}

impl Lowerer {
    /// A program never writes its input (question 35, fm3 log 101), and
    /// that holds through a call (fm3 log 106): `in$` handed to a
    /// function of the store as a parameter the function pushes into or
    /// ends, by its name or by handing it on in turn, is refused where
    /// it is handed, in a function's body, an initial value or a
    /// wiring. "Pushes into" is the scheduler's own walk (`Pushes`, log
    /// 78), which errs toward yes where a function can reach itself
    fn input_handed(&self, store: &Store) -> Result<(), Error> {
        let mut bodies: HashMap<String, Vec<(Vec<String>, &[Stmt])>> = HashMap::new();
        for f in &store.features {
            for d in &f.code.decls {
                if let Decl::Fn(fd) = d {
                    let mut params: Vec<String> = fd.results.iter().map(|p| p.name.clone()).collect();
                    params.extend(fd.params().map(|p| p.name.clone()));
                    bodies.entry(mangle(&fd.name)).or_default().push((params, fd.body.as_slice()));
                }
            }
        }
        let mut walker = Pushes { l: self, bodies, known: HashMap::new(), active: Names::new(), file: store.product_file.clone(), depth: 0, handed: None };
        let none = Names::new();
        for f in &store.features {
            if f.name == "platform" {
                continue;
            }
            for d in &f.code.decls {
                let (mut out, mut own) = (Names::new(), Names::new());
                match d {
                    Decl::Fn(fd) => {
                        let mut bound: Names = fd.results.iter().map(|p| p.name.clone()).collect();
                        bound.extend(fd.params().map(|p| p.name.clone()));
                        walker.of(&fd.body, &bound, &mut out, &mut own);
                    }
                    Decl::Var(v) => walker.init(v, &none, &mut out, &mut own),
                    Decl::Wire(e) => walker.expr(e, &none, &mut out, &mut own),
                    _ => {}
                }
                if let Some((line, to)) = walker.handed.take() {
                    return Err(lex::error(&f.code.file, line, format!("{}. Here it is given to '{}', which pushes into or ends the stream it is given", INPUT_REFUSED, to)));
                }
            }
        }
        Ok(())
    }
}

pub fn lower(store: &Store) -> Result<Lowered, Error> {
    let mut l = Lowerer { device_param: None, device_fns: HashMap::new(), trial: (int_ty(), float_ty()), funcs: Vec::new(), types: HashMap::new(), type_lines: Vec::new(), data: Vec::new(), out: String::new(), nstr: 0, fvars: Vec::new(), copies: std::collections::BTreeSet::new(), rings: std::collections::BTreeSet::new(), push_read: None, nodes: Vec::new(), node_inputs: std::collections::HashSet::new(), edges: Vec::new(), timed: std::collections::HashSet::new(), timed_all: false, kept: std::collections::HashSet::new(), kept_all: false, all_queues: false, queues: std::collections::HashSet::new(), queue_locals: std::collections::HashSet::new(), read_by_name: std::collections::HashSet::new(), node_reads: HashMap::new(), any_rated_wiring: false, regular: std::collections::HashSet::new(), regular_locals: std::collections::HashSet::new(), cur: String::new(), ranks: HashMap::new(), features: Vec::new(), parents: HashMap::new(), type_feature: HashMap::new(), round: Round::Any, candidate: None, product: HashMap::new(), statics: std::collections::HashSet::new(), rated: std::collections::HashSet::new(), rates: HashMap::new(), edge_fns: HashMap::new(), bare: std::collections::HashSet::new(), bare_edges: HashMap::new(), bare_gates: None, loose_push: false, after_push: None, on_beat: std::collections::HashSet::new(), loop_beats: HashMap::new(), loop_beat: None, clock: store.clock, static_schedule: false, wakes: HashMap::new(), rests: HashMap::new(), guard: true, push_site: None, sure_push: false, arrivals: HashMap::new(), ended: Vec::new(), queue_pushes: std::collections::BTreeMap::new(), written: std::collections::HashSet::new(), placed: std::collections::HashSet::new() };
    for f in &store.features {
        l.features.push(f.name.clone());
        l.ranks.insert(f.name.clone(), store.rank(f.layer.as_deref().unwrap_or("")));
        l.parents.insert(f.name.clone(), f.parent.clone());
        if store.marks.get(&f.name) == Some(&Mark::StaticOn) {
            l.statics.insert(f.name.clone());
        }
    }
    // types first, then every signature, then the variables (a wiring
    // names a task), so a body may use what a later feature declares
    for f in &store.features {
        l.cur = f.name.clone();
        for d in &f.code.decls {
            if let Decl::Type(t) = d {
                l.declare_type(t, &f.code.file)?;
            }
        }
    }
    for f in &store.features {
        l.cur = f.name.clone();
        for d in &f.code.decls {
            if let Decl::Fn(fd) = d {
                l.declare(fd, &f.name, &f.code.file)?;
            }
        }
    }
    l.name_methods(&store.features.iter().map(|f| (f.name.clone(), f.code.file.clone())).collect())?;
    // a `<<` method over a `char$` is lowered twice (log 87): as it is
    // written, over a stream, and again over the output device, where
    // `o$` has no value and every push into it is the platform's write.
    // The names are settled here, before a body is lowered, because a
    // call site may be reached before the method's own definition
    for i in 0..l.funcs.len() {
        let g = &l.funcs[i];
        if g.platform.is_some() || g.params.len() != 2 || g.params[0].1 != Ty::string() {
            continue;
        }
        if !matches!(g.parts.as_slice(), [NamePart::Group, NamePart::Sym(op), NamePart::Group] if op == "<<") {
            continue;
        }
        let (ir, item) = (g.ir.clone(), g.params[1].1.mangled());
        l.device_fns.insert(ir, crate::ssa::method_name("__out", &[item]));
    }
    // is any task wired at a rate (log 83)? looked for before any body
    // is lowered, since a task's body serves every wiring
    for f in &store.features {
        for d in &f.code.decls {
            let Decl::Var(v) = d else { continue };
            if matches!(&v.init, Some(Init::Value(e)) if is_rated_wiring(e)) {
                l.timed.insert(v.name.clone());
                l.kept.insert(v.name.clone());
            }
        }
    }
    l.any_rated_wiring = store.features.iter().any(|f| {
        f.code.decls.iter().any(|d| match d {
            Decl::Var(v) => matches!(&v.init, Some(Init::Value(e)) if is_rated_wiring(e)),
            Decl::Wire(e) => is_rated_wiring(e),
            Decl::Fn(fd) => wired_at_a_rate(&fd.body),
            _ => false,
        })
    });
    // which streams something asks a time of (log 73): looked for
    // before any ring is made
    for f in &store.features {
        for d in &f.code.decls {
            if let Decl::Fn(fd) = d {
                let params: Vec<String> = fd.params().filter(|p| p.seq).map(|p| p.name.clone()).collect();
                let mut w = Words { timed: std::mem::take(&mut l.timed), timed_all: l.timed_all, kept: std::mem::take(&mut l.kept), kept_all: l.kept_all, read: std::mem::take(&mut l.read_by_name) };
                time_words(&fd.body, &params, &mut w);
                l.timed = w.timed;
                l.timed_all = w.timed_all;
                l.kept = w.kept;
                l.kept_all = w.kept_all;
                l.read_by_name = w.read;
            }
        }
    }
    // a store nothing keeps history in is all queues (log 89): every
    // read may take the queue's word, which the push has already
    // proved the residency for
    l.all_queues = !l.kept_all && !l.timed_all && l.kept.is_empty() && l.timed.is_empty();
    // the product's settings name the store's functions (log 41)
    for (words, n) in &store.product {
        let key = words.join("_");
        if !l.funcs.iter().any(|g| g.key == key && g.platform.is_none()) {
            return Err(lex::error(&store.product_file, 0, format!("the product bounds '{}', which is no function of the store", words.join(" "))));
        }
        l.product.insert(key, *n);
    }
    for f in &store.features {
        l.cur = f.name.clone();
        for d in &f.code.decls {
            if let Decl::Var(v) = d {
                l.declare_var(v, &f.name, &f.code.file)?;
            }
        }
    }
    // a wiring with nothing to fill makes its function a sink (log 57)
    for f in &store.features {
        l.cur = f.name.clone();
        for d in &f.code.decls {
            if let Decl::Wire(e) = d {
                l.mark_sink(e, &f.name, &f.code.file)?;
            }
        }
    }
    // which streams have no storage (question 50): settled before an
    // edge is collected, since an edge out of one is not a node
    l.settle_bare(store)?;
    for f in &store.features {
        l.cur = f.name.clone();
        for d in &f.code.decls {
            match d {
                Decl::Var(v) => l.collect_nodes(v, &f.name, &f.code.file)?,
                Decl::Wire(e) => l.collect_wire(e, &f.name, &f.code.file)?,
                Decl::Edge { target, items, cond, line } => l.collect_edge(target, items, cond.as_ref(), *line, &f.name, &f.code.file)?,
                _ => {}
            }
        }
    }
    // the arrival bound (log 79): the most items one push statement
    // from a plain function or the reset pushes into each feature-scope
    // stream, counted before anything is lowered
    {
        let mut counts: HashMap<String, Option<i64>> = HashMap::new();
        let bytes = |n: &str| l.fvar(n).map(|f| matches!(&f.ty, Ty::Stream(e) if e.ir() == "u8"));
        for f in &store.features {
            for d in &f.code.decls {
                match d {
                    Decl::Fn(fd) if !fd.task => {
                        let mut bound: Names = fd.results.iter().map(|p| p.name.clone()).collect();
                        bound.extend(fd.params().map(|p| p.name.clone()));
                        arrivals(&fd.body, &bound, &bytes, &mut counts);
                    }
                    Decl::Var(v) => {
                        if let Some(Init::Pushes { items, cond }) = &v.init {
                            if let Some(b) = bytes(&v.name) {
                                arrival(&v.name, items, cond.as_ref(), b, &mut counts);
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
        l.arrivals = counts;
    }
    // the input device handed to what would write it (question 35)
    l.input_handed(store)?;
    l.emit_context(store)?;
    // which pushes are on their stream's beat already (question 56):
    // settled once the nodes and the rates are, before any body
    let (on_beat, loop_beats) = Beat::new(&l, store).solve(store);
    l.on_beat = on_beat;
    l.loop_beats = loop_beats;
    for f in &store.features {
        l.cur = f.name.clone();
        writeln!(l.out, "\n; feature {} (layer {})", f.name, f.layer.as_deref().unwrap_or("")).unwrap();
        for d in &f.code.decls {
            if let Decl::Fn(fd) = d {
                l.lower_fn(fd, &f.name, &f.code.file)?;
            }
        }
        // the feature's edges (log 72), sinks the front end wrote
        let edges = std::mem::take(&mut l.edges);
        for (fd, feature, file) in &edges {
            if feature == &f.name {
                l.lower_fn(fd, feature, file)?;
            }
        }
        l.edges = edges;
    }
    l.emit_links();
    if !l.rings.is_empty() {
        writeln!(l.out, "\n; a stream's storage, carved from the arena: cap items ({} unless more are) — a ring keeps them in twice that many slots, each item in both halves so that every window is one view (log 65), and a tick per item unless regular; a queue keeps them in a plain run of cap slots, item k at k minus the origin (log 89) — and a reader's view at its start", RING_ITEMS).unwrap();
    }
    for (t, maker) in &l.rings {
        let ticks = if maker == "stream" { "    tbytes: i64 = mul slots, 8\n    ttotal: i64 = add tbytes, 16\n    tb: ptr = arena_alloc(a, ttotal)\n    buffer_init(tb, 8, slots)\n".to_string() } else { String::new() };
        let init = match maker.as_str() {
            "queue" => "ring_queue(r, vb, hz, cap)",
            "regular" => "ring_regular(r, vb, hz, 1, 0)",
            _ => "ring_init(r, vb, tb, hz)",
        };
        // a ring's buffer is twice its resident count, each item stored
        // in both halves (log 65); a queue's is a plain run of slots
        let (line, slots) = if maker == "queue" { ("", "cap") } else { ("    slots: i64 = mul cap, 2\n", "slots") };
        // a queue's header is a word longer: where slot 0 is (fm3 log 115)
        let header = if maker == "queue" { 72 } else { 64 };
        writeln!(l.out, "fn __{}_{}(hz: i64, cap: i64) -> {}$\n    a: ptr = addr __arena\n    r: ptr = arena_alloc(a, {})\n{}    sz: i64 = sizeof {}\n    bytes: i64 = mul sz, {}\n    total: i64 = add bytes, 16\n    vb: ptr = arena_alloc(a, total)\n    buffer_init(vb, sz, {})\n{}    {}\n    s: {}$ = stream r\n    ret s", maker, t, t, header, line, t, slots, slots, ticks, init, t).unwrap();
    }
    if !l.copies.is_empty() {
        writeln!(l.out, "\n; a view's items as a new stream (log 38): what `frame`, `behind`, `from ... to` and a string literal give; stamped once where something asks its time (log 73)").unwrap();
    }
    for (t, maker) in &l.copies {
        let push = if maker == "queue" { "push_queue(s, x)" } else { "push s, x" };
        if maker != "stream" {
            writeln!(l.out, "fn __copy_{}{}(v: {}[]) -> {}$\n    n: i64 = len v\n    least: i64 = const {}\n    cap: i64 = max(n, least)\n    s: {}$ = __{}_{}({}, cap)\n    loop(i: i64 = 0)\n        done: u1 = cmp.ge i, n\n        if done\n            break\n        x: {} = load v, i\n        {}\n        i2: i64 = add i, 1\n        continue i2\n    ret s", copy_infix(maker), t, t, t, RING_ITEMS, t, maker, t, CLOCK_HZ, t, push).unwrap();
        } else {
            writeln!(l.out, "fn __copy_timed_{}(v: {}[]) -> {}$\n    n: i64 = len v\n    least: i64 = const {}\n    cap: i64 = max(n, least)\n    s: {}$ = __stream_{}({}, cap)\n    t: i64 = __now()\n    loop(i: i64 = 0)\n        done: u1 = cmp.ge i, n\n        if done\n            break\n        x: {} = load v, i\n        push s, t, x\n        i2: i64 = add i, 1\n        continue i2\n    ret s", t, t, t, RING_ITEMS, t, t, CLOCK_HZ, t).unwrap();
        }
    }
    let mut ir = String::new();
    writeln!(ir, "; lowered from the zero store {}", store.path.display()).unwrap();
    // the platform's push of an arriving byte is a system stream's too
    ir.push_str(&if l.all_queues || l.queues.contains("in") { PRELUDE.replace("__push(s, c)", "push_queue(s, c)") } else { PRELUDE.to_string() });
    // the one function that differs per clock (log 77)
    ir.push_str(if store.clock == super::store::Clock::Real { REAL_CLOCK } else { VIRTUAL_CLOCK });
    ir.push_str(if l.rings.iter().any(|(_, m)| m == "stream") {
        PUSH_BRANCHED
    } else if l.all_queues {
        PUSH_QUEUE
    } else {
        PUSH_PLAIN
    });
    if !l.type_lines.is_empty() {
        ir.push('\n');
        for t in &l.type_lines {
            ir.push_str(t);
            ir.push('\n');
        }
    }
    if !l.data.is_empty() {
        ir.push('\n');
        for d in &l.data {
            ir.push_str(d);
            ir.push('\n');
        }
    }
    ir.push_str(&l.out);
    // a `platform` body of the store's own is IR the front end does not
    // read, and may end anything
    let unread = store.features.iter().any(|f| f.name != "platform" && f.code.decls.iter().any(|d| matches!(d, Decl::Fn(fd) if !fd.platform.is_empty())));
    let ir = settle_pushes(ir, &l.queue_pushes, &l.ended, unread);
    let ir = settle_context(&ir, &l.written, unread);
    let ir = settle_counts(&ir);
    // the text carries what the store reaches (log 70): every function
    // of the store's own features, the platform feature's that a case
    // names, and the runner's entries are roots
    let mut roots: std::collections::HashSet<String> = l.funcs.iter().filter(|f| f.feature != "platform").map(|f| f.ir.clone()).collect();
    for f in &store.features {
        roots.insert(format!("__set___enabled_{}", f.name));
        for c in &f.cases {
            if let ExprKind::Phrase(parts) = &c.call.kind {
                if let Ok((cands, _)) = find_methods(&l.funcs, parts, &|_| false, &f.md_file, c.line) {
                    roots.extend(cands.iter().map(|g| g.ir.clone()));
                }
            }
        }
    }
    for entry in ["__zero_reset", "__zero_start", "__out_len", "__out_byte", "__in_ch"] {
        roots.insert(entry.to_string());
    }
    // a store with a case that asserts on time has the marks' reader
    // (fm3 log 91, 95)
    if store.features.iter().any(|f| f.cases.iter().any(|c| matches!(c.expect, Expect::Timed(_)))) {
        roots.insert("__out_mark".to_string());
    }
    let mut pruned = prune(&ir, &roots);
    // ... and a store where nothing waits and no case reads a mark
    // keeps no marks, so its reset does not clear them
    if !pruned.contains("\nfn __wait(") && !pruned.contains("\nfn __out_mark(") {
        let reset: String = MARKS_RESET.iter().map(|l| format!("    {}\n", l)).collect();
        pruned = prune(&ir.replacen(&reset, "", 1), &roots);
    }
    let ir = pruned;
    Ok(Lowered { ir, funcs: l.funcs, features: l.features, marks: store.marks.clone() })
}

/// Each queue's push takes its word (fm3 log 108). `ended` is written
/// by `end` alone, and a ring made for a type is only ever named by
/// that type or by an abstract type it fits, the IR making the
/// instance. So a push through a name of element type T can find its
/// stream ended only where some `end` in the store has an operand of
/// element type E with T fitting E, E fitting T, or the two one type in
/// the IR, as `char` and `uint8` are; every other push is
/// `push_queue_open`, which does not ask. `ended` holds every `end` the
/// lowering wrote, a trial's and a pruned function's among them, so it
/// errs toward the check, and `unread` keeps every check
fn settle_pushes(mut ir: String, pushes: &std::collections::BTreeMap<String, Ty>, ended: &[Ty], unread: bool) -> String {
    for (key, t) in pushes {
        let reached = unread || ended.iter().any(|e| fits(t, e) || fits(e, t) || e.ir() == t.ir());
        ir = ir.replace(&format!("push_queue<{}>(", key), if reached { "push_queue(" } else { "push_queue_open(" });
    }
    ir
}

/// The context reached once (fm3 log 110), settled in the finished text
/// because a write may be lowered after the read it matters to. Two
/// things, a function at a time. A function that names `_this` gets the
/// context's address as its first line, the one place it is formed. And
/// a field nothing in the store writes is fetched once: where a read of
/// it is already in hand in a block still open, above, a later read
/// into a temporary is dropped and its name replaced by the earlier
/// one. The open blocks are the text's indentation: a line less
/// indented than a read closes the block that read was in. Nothing is
/// asked about what lies between the two reads, there being no store to
/// such a field in any function; `unread`, a `platform` body of the
/// store's own, which may call a setter, reuses nothing
fn settle_context(ir: &str, written: &std::collections::HashSet<String>, unread: bool) -> String {
    let names = words;
    let lines: Vec<&str> = ir.lines().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < lines.len() {
        out.push_str(lines[i]);
        out.push('\n');
        if !lines[i].starts_with("fn ") {
            i += 1;
            continue;
        }
        let start = i + 1;
        let mut end = start;
        while end < lines.len() && lines[end].starts_with(' ') {
            end += 1;
        }
        let body = &lines[start..end];
        i = end;
        let mut uses = false;
        let mut made = false;
        for l in body {
            names(l, &mut |w| {
                uses |= w == THIS;
                None
            });
            made |= l.trim_start().starts_with("_this:");
        }
        if !uses {
            for l in body {
                out.push_str(l);
                out.push('\n');
            }
            continue;
        }
        // the loads of the context, by the name each defines: the line,
        // and how many lines kept still use it
        let mut loads: HashMap<String, (usize, usize)> = HashMap::new();
        // the fields in hand: how far in the read stands, the field, its name
        let mut held: Vec<(usize, String, String)> = Vec::new();
        let mut renamed: HashMap<String, String> = HashMap::new();
        let mut dropped = vec![false; body.len()];
        for (k, l) in body.iter().enumerate() {
            let t = l.trim_start();
            let indent = l.len() - t.len();
            held.retain(|(d, _, _)| *d <= indent);
            if let Some(name) = t.strip_suffix(&format!(": __ctx = load {}", THIS)) {
                loads.insert(name.to_string(), (k, 0));
                continue;
            }
            let read = t.split_once(" = get ").and_then(|(def, rhs)| {
                let (c, f) = rhs.split_once(", ")?;
                let (name, _) = def.split_once(": ")?;
                loads.contains_key(c).then(|| (name.to_string(), f.to_string()))
            });
            if let Some((name, f)) = read {
                if !unread && !written.contains(&f) {
                    let temporary = name.len() > 1 && name.starts_with('_') && name[1..].bytes().all(|c| c.is_ascii_digit());
                    match held.iter().find(|(_, g, _)| *g == f) {
                        Some((_, _, earlier)) if temporary => {
                            renamed.insert(name, earlier.clone());
                            dropped[k] = true;
                            continue;
                        }
                        Some(_) => {}
                        None => held.push((indent, f, name)),
                    }
                }
            }
            names(t, &mut |w| {
                if let Some(u) = loads.get_mut(w) {
                    u.1 += 1;
                }
                None
            });
        }
        for (k, n) in loads.values() {
            if *n == 0 {
                dropped[*k] = true;
            }
        }
        if !made {
            writeln!(out, "    {}: ptr = addr __ctx_mem", THIS).unwrap();
        }
        for (k, l) in body.iter().enumerate() {
            if dropped[k] {
                continue;
            }
            if renamed.is_empty() {
                out.push_str(l);
            } else {
                out.push_str(&names(l, &mut |w| renamed.get(w).cloned()));
            }
            out.push('\n');
        }
    }
    out
}

/// `count` asked once (fm3 log 112), settled in the finished text. In
/// one function, `X: i64 = count R` is dropped and `X` is the `Y` of an
/// earlier `Y: i64 = count R` where: `R` is the same value; the first
/// asking is a statement of a block the second is inside, so every way
/// to the second passes the first; and no line between could push.
/// Between is every line after the first and before the second, and,
/// where the second stands in a loop the first does not, that loop's
/// whole body, a pass coming round to the second without the first. A
/// line is harmless by `quiet`: a list of what is allowed, so a word
/// nobody thought of gives up. One kind of line is passed over, an `if`
/// that is a statement of the first asking's own block, with no `else`,
/// whose last line leaves the pass or the function: whoever enters it
/// reaches the second asking again only through the first. The `conv`
/// that follows the asking goes with it. A function of the store is
/// harmless to call where its own body, and the body of all it calls,
/// is quiet lines and nothing else, and it has no rule for a target
fn settle_counts(ir: &str) -> String {
    let lines: Vec<&str> = ir.lines().collect();
    let indent = |l: &str| l.len() - l.trim_start().len();
    // the functions: name, the body's lines, whether a target has a rule for it
    let mut fns: Vec<(&str, usize, usize, bool)> = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let Some(sig) = lines[i].strip_prefix("fn ") else {
            i += 1;
            continue;
        };
        let name = sig.split('(').next().unwrap_or("");
        let mut end = i + 1;
        while end < lines.len() && lines[end].starts_with(' ') {
            end += 1;
        }
        fns.push((name, i + 1, end, lines.get(end).is_some_and(|l| l.starts_with("platform "))));
        i = end;
    }
    // the functions that are harmless to call: the least set that holds
    let mut still: std::collections::HashSet<&str> = std::collections::HashSet::new();
    loop {
        let before = still.len();
        for &(name, _, _, _) in &fns {
            if !still.contains(name) && fns.iter().filter(|f| f.0 == name).all(|&(_, a, b, ruled)| !ruled && lines[a..b].iter().all(|l| quiet(l, &still))) {
                still.insert(name);
            }
        }
        if still.len() == before {
            break;
        }
    }
    let asking = |l: &str| -> Option<(String, String)> {
        let (def, r) = l.trim_start().split_once(" = count ")?;
        let x = def.strip_suffix(": i64")?;
        (!r.contains(' ') && !x.contains(' ')).then(|| (x.to_string(), r.to_string()))
    };
    let conv = |l: &str| -> Option<(String, String, String)> {
        let (def, v) = l.trim_start().split_once(" = conv ")?;
        let (p, t) = def.split_once(": ")?;
        Some((p.to_string(), t.to_string(), v.to_string()))
    };
    let temporary = |n: &str| n.len() > 1 && n.starts_with('_') && n[1..].bytes().all(|c| c.is_ascii_digit());
    let mut dropped = vec![false; lines.len()];
    let mut renamed: Vec<HashMap<String, String>> = Vec::new();
    for &(_, a, b, _) in &fns {
        let mut names: HashMap<String, String> = HashMap::new();
        // the askings in blocks still open: the line, how far in, the reader, the count
        let mut asked: Vec<(usize, usize, String, String)> = Vec::new();
        for k in a..b {
            let d = indent(lines[k]);
            asked.retain(|q| q.1 <= d);
            let Some((x, r)) = asking(lines[k]) else { continue };
            let first = asked.iter().rev().find(|q| q.2 == r && {
                // the outermost loop the second asking is in and the first is not
                let lp = (q.0 + 1..k).find(|&m| head(lines[m]) == "loop" && indent(lines[m]) >= q.1 && (m + 1..=k).all(|n| indent(lines[n]) > indent(lines[m])));
                let stop = lp.unwrap_or(k);
                let mut clear = true;
                let mut m = q.0 + 1;
                while clear && m < stop {
                    if indent(lines[m]) == q.1 && head(lines[m]) == "if" && lines[m].trim_start().starts_with("if ") {
                        let e = (m + 1..b).find(|&n| indent(lines[n]) <= q.1).unwrap_or(b);
                        let leaves = e - 1 > m && indent(lines[e - 1]) == q.1 + 4 && matches!(head(lines[e - 1]), "break" | "continue" | "ret");
                        if leaves && e <= stop && lines.get(e).is_none_or(|l| l.trim() != "else") {
                            m = e;
                            continue;
                        }
                    }
                    clear = quiet(lines[m], &still);
                    m += 1;
                }
                if let Some(lp) = lp {
                    let e = (lp + 1..b).find(|&n| indent(lines[n]) <= indent(lines[lp])).unwrap_or(b);
                    clear = clear && lines[lp..e].iter().all(|l| quiet(l, &still));
                }
                clear
            });
            match first {
                Some(q) if temporary(&x) => {
                    dropped[k] = true;
                    // ... and the conversion of the count with it
                    if let (Some((p, t, v)), Some((p0, t0, v0))) = (lines.get(k + 1).filter(|_| k + 1 < b).and_then(|l| conv(l)), conv(lines[q.0 + 1])) {
                        if v == x && v0 == q.3 && t == t0 && temporary(&p) && indent(lines[q.0 + 1]) == q.1 {
                            dropped[k + 1] = true;
                            names.insert(p, p0);
                        }
                    }
                    names.insert(x, q.3.clone());
                }
                _ => asked.push((k, d, r, x)),
            }
        }
        renamed.push(names);
    }
    let mut out = String::new();
    let mut f = 0;
    for (k, l) in lines.iter().enumerate() {
        while f < fns.len() && k >= fns[f].2 {
            f += 1;
        }
        if dropped[k] {
            continue;
        }
        match fns.get(f) {
            Some(&(_, a, _, _)) if k >= a && !renamed[f].is_empty() => out.push_str(&words(l, &mut |w| renamed[f].get(w).cloned())),
            _ => out.push_str(l),
        }
        out.push('\n');
    }
    out
}

/// a line's operation: the first word after its definitions, or its first word
fn head(line: &str) -> &str {
    let t = line.trim_start();
    let rhs = match t.split_once(" = ") {
        Some((defs, r)) if defs.contains(": ") && !defs.contains('(') => r,
        _ => t,
    };
    rhs.split(|c: char| c == ' ' || c == '(').next().unwrap_or("")
}

/// Could this line push into a stream, end one, or write memory? Not
/// where its operation is on the list and every call on it is to a
/// reading word of the library or a function of `still` (fm3 log 112)
fn quiet(line: &str, still: &std::collections::HashSet<&str>) -> bool {
    const PURE: [&str; 15] = ["const", "conv", "get", "set", "pack", "add", "sub", "mul", "and", "or", "xor", "load", "addr", "len", "count"];
    const FLOW: [&str; 7] = ["if", "else", "loop", "break", "continue", "yield", "ret"];
    const READS: [&str; 5] = ["peek_queue", "peek", "ended", "received", "advance"];
    let h = head(line);
    if !(PURE.contains(&h) || FLOW.contains(&h) || h.starts_with("cmp.") || READS.contains(&h) || still.contains(h)) {
        return false;
    }
    let mut calls = true;
    let bytes = line.as_bytes();
    words(line, &mut |w| {
        let after = w.as_ptr() as usize - line.as_ptr() as usize + w.len();
        if bytes.get(after) == Some(&b'(') && !(w == "loop" || READS.contains(&w) || still.contains(w)) {
            calls = false;
        }
        None
    });
    calls
}

/// a line with each of its words given to `f`, and replaced where `f` says
fn words(line: &str, f: &mut dyn FnMut(&str) -> Option<String>) -> String {
    let word = |c: char| c.is_ascii_alphanumeric() || c == '_';
    let mut out = String::new();
    let mut rest = line;
    while !rest.is_empty() {
        let n = rest.find(|c: char| !word(c)).unwrap_or(rest.len());
        if n == 0 {
            let c = rest.chars().next().unwrap();
            out.push(c);
            rest = &rest[c.len_utf8()..];
        } else {
            out.push_str(&f(&rest[..n]).unwrap_or_else(|| rest[..n].to_string()));
            rest = &rest[n..];
        }
    }
    out
}

/// in a chain, the name feature i's body is emitted under: `key__f`, or
/// the plain name when it is the newest and static, since no link
/// stands above it (log 71)
fn body_name(info: &FnInfo, i: usize, statics: &std::collections::HashSet<String>) -> String {
    named_body(&info.ir, &info.plain, &info.chain, i, statics)
}

/// the same over any base name: the device copies of a `<<` method
/// chain under their own name, `__out__int` and `__out__int__watch`
/// (log 90)
fn named_body(ir: &str, plain: &str, chain: &[String], i: usize, statics: &std::collections::HashSet<String>) -> String {
    if i + 1 == chain.len() && statics.contains(&chain[i]) {
        plain.to_string()
    } else {
        format!("{}__{}", ir, chain[i])
    }
}

fn named_link(ir: &str, plain: &str, chain: &[String], i: usize, statics: &std::collections::HashSet<String>) -> String {
    if statics.contains(&chain[i]) {
        named_body(ir, plain, chain, i, statics)
    } else if i + 1 == chain.len() {
        plain.to_string()
    } else {
        format!("{}__before_{}", ir, chain[i + 1])
    }
}

/// what a call from above reaches at feature i of a chain: the link
/// that gates on i's switch, or i's body itself when i is static
fn link_name(info: &FnInfo, i: usize, statics: &std::collections::HashSet<String>) -> String {
    named_link(&info.ir, &info.plain, &info.chain, i, statics)
}

/// the emitted text without the functions nothing reaches from the
/// roots, and the `data` strings nothing names (log 70). A top-level
/// item is a `fn` with its indented lines and any `platform` block
/// after it, a `type` or a `data`; a function reaches another by a
/// call written `name(` — the emitted functions are only ever called
/// so, and an operation form names the IR's library, which is not in
/// the text. The comment and blank lines above an item go with it; a
/// `; feature` heading stays
fn prune(ir: &str, roots: &std::collections::HashSet<String>) -> String {
    // the items: (the loose lines above, the item's lines, its name if a fn or data)
    let mut items: Vec<(Vec<&str>, Vec<&str>, Option<(bool, String)>)> = Vec::new();
    let mut loose: Vec<&str> = Vec::new();
    for line in ir.lines() {
        let top = !line.is_empty() && !line.starts_with(' ') && !line.starts_with(';');
        if top && !line.starts_with("platform ") {
            let word = line.split(|c: char| c == ' ' || c == '(' || c == ':').next().unwrap_or("");
            let name = line[word.len()..].trim_start().split(|c: char| c == '(' || c == ':' || c == ' ' || c == '=').next().unwrap_or("").to_string();
            let what = match word {
                "fn" => Some((true, name)),
                "data" => Some((false, name)),
                _ => None,
            };
            items.push((std::mem::take(&mut loose), vec![line], what));
        } else if top || line.starts_with(' ') {
            match items.last_mut() {
                Some(item) if loose.is_empty() => item.1.push(line),
                _ => loose.push(line),
            }
        } else {
            loose.push(line);
        }
    }
    // reachability over the functions
    let calls = |text: &[&str]| -> Vec<String> {
        let mut out = Vec::new();
        for line in text.iter().skip(1) {
            let bytes = line.as_bytes();
            let mut i = 0;
            while i < bytes.len() {
                if bytes[i].is_ascii_alphabetic() || bytes[i] == b'_' {
                    let start = i;
                    while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                        i += 1;
                    }
                    if i < bytes.len() && bytes[i] == b'(' {
                        out.push(line[start..i].to_string());
                    }
                } else {
                    i += 1;
                }
            }
        }
        out
    };
    // a name may be a method set's, several functions under it
    let mut index: HashMap<&str, Vec<usize>> = HashMap::new();
    for (i, it) in items.iter().enumerate() {
        if let Some((true, n)) = &it.2 {
            index.entry(n.as_str()).or_default().push(i);
        }
    }
    let mut kept = vec![false; items.len()];
    let mut work: Vec<usize> = roots.iter().filter_map(|r| index.get(r.as_str())).flatten().copied().collect();
    while let Some(i) = work.pop() {
        if kept[i] {
            continue;
        }
        kept[i] = true;
        for c in calls(&items[i].1) {
            if let Some(js) = index.get(c.as_str()) {
                work.extend(js.iter().copied().filter(|&j| !kept[j]));
            }
        }
    }
    // a type stays; a data item stays when a kept function names it
    let mut text = String::new();
    for (i, it) in items.iter().enumerate() {
        if let Some((false, _)) = &it.2 {
            continue;
        }
        if it.2.is_none() || kept[i] {
            text.push_str(&it.1.join("\n"));
            text.push('\n');
        }
    }
    for (i, it) in items.iter().enumerate() {
        if let Some((false, n)) = &it.2 {
            kept[i] = text.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_')).any(|w| w == n);
        }
    }
    let mut out = String::new();
    for (i, it) in items.iter().enumerate() {
        let keep = it.2.is_none() || kept[i];
        let heading: Vec<&str> = it.0.iter().copied().filter(|l| l.starts_with("; feature ")).collect();
        if keep {
            for l in &it.0 {
                out.push_str(l);
                out.push('\n');
            }
            for l in &it.1 {
                out.push_str(l);
                out.push('\n');
            }
        } else if !heading.is_empty() {
            out.push('\n');
            for l in heading {
                out.push_str(l);
                out.push('\n');
            }
        }
    }
    for l in loose {
        out.push_str(l);
        out.push('\n');
    }
    out
}

/// resolve a `## testing` case against the lowered store
pub fn resolve_case(lowered: &Lowered, case: &Case, file: &str, int_bits: u32) -> Result<Call, Error> {
    let ExprKind::Phrase(parts) = &case.call.kind else {
        return Err(lex::error(file, case.line, "a case calls a function"));
    };
    let (cands, args) = find_methods(&lowered.funcs, parts, &|_| false, file, case.line)?;
    let cands: Vec<FnInfo> = cands.into_iter().cloned().collect();
    // a case's arguments are literals: a method takes them when each is
    // a number where a number is wanted, a bool where a bool is; a
    // number is an `int` first, then the width the case runs at — a
    // case is the runner's, not the IR's, so it knows the path's width
    // (log 52) — then any number type
    let width = Ty::Num(format!("i{}", int_bits));
    let takes = |a: &Expr, ty: &Ty, round: Round| match (&a.kind, ty) {
        (ExprKind::Int(_), Ty::Num(n)) => match round {
            Round::Own => fits(&int_ty(), &Ty::Num(n.clone())),
            Round::Product => fits(&width, &Ty::Num(n.clone())),
            Round::Any => true,
        },
        (ExprKind::Int(_), Ty::Enum(_)) | (ExprKind::Bool(_), Ty::Bool) => true,
        _ => false,
    };
    let mut applicable = Vec::new();
    for round in [Round::Own, Round::Product, Round::Any] {
        applicable = (0..cands.len()).filter(|&i| args.iter().zip(&cands[i].params).all(|(a, (_, t))| takes(a, t, round))).collect();
        if !applicable.is_empty() {
            break;
        }
    }
    if applicable.is_empty() {
        return Err(lex::error(file, case.line, "a case's arguments are numbers"));
    }
    let info = match pick(&cands, &applicable) {
        Ok(i) => &cands[i],
        Err(amb) => return Err(ambiguous(&cands, &amb, file, case.line)),
    };
    if info.task {
        return Err(lex::error(file, case.line, format!("'{}' is a task: a case calls a function that reads its stream", info.key)));
    }
    let mut vals = Vec::new();
    for a in &args {
        let v = match &a.kind {
            ExprKind::Int(v) => *v,
            ExprKind::Bool(b) => *b as i64,
            _ => unreachable!(),
        };
        vals.push(v);
    }
    for (feature, on) in &case.context {
        let clause = format!("`with {} {}`", feature, if *on { "on" } else { "off" });
        match lowered.marks.get(feature) {
            Some(Mark::StaticOff) => return Err(lex::error(file, case.line, format!("{}: {} is static off in the product, so its code and its cases are not in the program", clause, feature))),
            Some(Mark::StaticOn) => return Err(lex::error(file, case.line, format!("{}: {} is static on in the product and cannot be switched", clause, feature))),
            _ => {}
        }
        if !lowered.features.contains(feature) {
            return Err(lex::error(file, case.line, format!("{}: no feature named '{}' in the store", clause, feature)));
        }
    }
    Ok(Call { func: info.ir.clone(), args: vals, nrets: info.results.len(), expect: case.expect.clone(), context: case.context.clone(), input: case.input.clone().unwrap_or_default().into_bytes() })
}

/// the refusal of an ambiguous call, naming the methods that contend
fn ambiguous(cands: &[FnInfo], amb: &[usize], file: &str, line: usize) -> Error {
    let names: Vec<String> = amb.iter().map(|&i| spelled(&cands[i])).collect();
    lex::error(file, line, format!("ambiguous: {} all take these arguments and none is the most specific; convert an argument, or declare a method for these types", names.join(" and ")))
}

/// The methods a phrase may name, and its arguments in order: the
/// bracket groups and bare values are arguments, and every word is
/// part of the name — unless no function reads that way, when a word
/// that names a variable in scope is an argument too (log 17). A name
/// is a set of methods (section 6, log 36): every method with these
/// words and this many parameters is returned, and the caller picks
/// one by the arguments' types with `pick`
fn find_methods<'a>(funcs: &'a [FnInfo], parts: &[Part], is_var: &dyn Fn(&str) -> bool, file: &str, line: usize) -> Result<(Vec<&'a FnInfo>, Vec<Expr>), Error> {
    let read = |vars_are_args: bool| -> Result<(Vec<NamePart>, Vec<Expr>), Error> {
        let mut name = Vec::new();
        let mut args = Vec::new();
        for p in parts {
            match p {
                Part::Word(w) if vars_are_args && is_var(w) => {
                    name.push(NamePart::Group);
                    args.push(Expr { kind: ExprKind::Name(w.clone()), line });
                }
                Part::Word(w) => name.push(NamePart::Word(w.clone())),
                Part::Args(list) => {
                    name.push(NamePart::Group);
                    for a in list {
                        if a.name.is_some() {
                            return Err(lex::error(file, line, "a call's arguments are by position"));
                        }
                        args.push(a.value.clone());
                    }
                }
                Part::Value(e) => {
                    name.push(NamePart::Group);
                    args.push(e.clone());
                }
            }
        }
        Ok((name, args))
    };
    let named = |name: &[NamePart]| -> Vec<&'a FnInfo> {
        let key = mangle(name);
        funcs.iter().filter(|f| f.key == key && !f.parts.iter().any(|p| matches!(p, NamePart::Sym(_)))).collect()
    };
    let (name, args) = read(false)?;
    let found: Vec<&FnInfo> = named(&name).into_iter().filter(|f| f.params.len() == args.len()).collect();
    if !found.is_empty() {
        return Ok((found, args));
    }
    let (name, args) = read(true)?;
    let all = named(&name);
    let found: Vec<&FnInfo> = all.iter().copied().filter(|f| f.params.len() == args.len()).collect();
    if !found.is_empty() {
        return Ok((found, args));
    }
    let Some(first) = all.first() else {
        let words: Vec<String> = name.iter().filter_map(|p| if let NamePart::Word(w) = p { Some(w.clone()) } else { None }).collect();
        return Err(lex::error(file, line, format!("no function named '{}'", words.join(" "))));
    };
    let mut arities: Vec<usize> = all.iter().map(|f| f.params.len()).collect();
    arities.sort();
    arities.dedup();
    let takes: Vec<String> = arities.iter().map(|n| n.to_string()).collect();
    Err(lex::error(file, line, format!("'{}' takes {} argument(s), given {}", first.key, takes.join(" or "), args.len())))
}

/// Multiple dispatch (section 6, log 36): among the methods that take
/// the arguments (`applicable`, indices into `cands`), the most
/// specific wins — the one whose every parameter type fits the
/// corresponding parameter of each of the others, `int32` before
/// `int` before `number`. With no such method the call is ambiguous,
/// and Err holds the contenders
fn pick(cands: &[FnInfo], applicable: &[usize]) -> Result<usize, Vec<usize>> {
    let at_least = |a: &FnInfo, b: &FnInfo| a.params.iter().zip(&b.params).all(|((_, x), (_, y))| fits(x, y));
    let minimal: Vec<usize> = applicable
        .iter()
        .copied()
        .filter(|&i| !applicable.iter().any(|&j| j != i && at_least(&cands[j], &cands[i]) && !at_least(&cands[i], &cands[j])))
        .collect();
    match minimal.as_slice() {
        [one] => Ok(*one),
        _ => Err(minimal),
    }
}

/// a literal's own type: `int` for `3`, `float` for `2.5`, `int$` for
/// `[1, 2]`; a range is a sequence of ints. `int` and `float` are what
/// an integer and a decimal literal are taken as: the abstract types,
/// or a width being tried (log 52)
fn literal_default(e: &Expr, int: &Ty, float: &Ty) -> Option<Ty> {
    match &e.kind {
        ExprKind::Int(_) => Some(int.clone()),
        ExprKind::Float(_) => Some(float.clone()),
        ExprKind::Neg(x) => literal_default(x, int, float),
        ExprKind::Range { .. } => Some(Ty::Stream(Box::new(int.clone()))),
        ExprKind::List(items) => {
            let first = literal_default(items.first()?, int, float)?;
            if items.iter().all(|i| literal_default(i, int, float).as_ref() == Some(&first)) {
                Some(Ty::Stream(Box::new(first)))
            } else {
                None
            }
        }
        _ => None,
    }
}

/// a method as a message names it: its words and its parameter types
/// in zero's spelling, `describe (int32)`
fn spelled(info: &FnInfo) -> String {
    let tys: Vec<String> = info.params.iter().map(|(_, t)| zero_ty(t)).collect();
    format!("{} ({})", spoken(info), tys.join(", "))
}

/// a type in zero's spelling, for messages
fn zero_ty(t: &Ty) -> String {
    match t {
        Ty::Bool => "bool".into(),
        Ty::Char => "char".into(),
        Ty::Num(n) => match n.as_str() {
            "u8" => "uint8".into(),
            "bf16" => "bfloat16".into(),
            n if n.len() > 1 && n[1..].parse::<u32>().is_ok() => match &n[..1] {
                "i" => format!("int{}", &n[1..]),
                "u" => format!("uint{}", &n[1..]),
                "f" => format!("float{}", &n[1..]),
                _ => n.to_string(),
            },
            n => n.to_string(),
        },
        Ty::Stream(e) if **e == Ty::Char => "string".into(),
        Ty::Stream(e) => format!("{}$", zero_ty(e)),
        Ty::Struct(n) | Ty::Enum(n) => n.clone(),
        Ty::None => String::new(),
    }
}

struct Lowerer {
    funcs: Vec<FnInfo>,
    types: HashMap<String, TypeInfo>,
    /// the `type` lines, in declaration order
    type_lines: Vec<String>,
    /// `data` lines for the string literals met so far
    data: Vec<String>,
    out: String,
    nstr: usize,
    /// the feature-scope variables, in composition order
    fvars: Vec<FVar>,
    /// the element types views were copied into streams of: one
    /// `__copy_T` each
    copies: std::collections::BTreeSet<(String, String)>,
    /// the stores made: element type and the maker's word, one
    /// `__stream_T`, `__regular_T` or `__queue_T` each (log 89)
    rings: std::collections::BTreeSet<(String, String)>,
    /// inside a push chain: what the stream's own name reads as
    push_read: Option<(String, PushRead)>,
    /// the wirings at feature scope, in declaration order
    nodes: Vec<Node>,
    /// the feature-scope streams some node reads: a push into one from a
    /// plain function is followed by `__run()`
    node_inputs: std::collections::HashSet<String>,
    /// the edges (log 72): the sink the front end wrote for each, its
    /// feature and its file, lowered with that feature's functions
    edges: Vec<(FnDecl, String, String)>,
    /// the streams something asks a time of (log 73): the names a time
    /// word is applied to anywhere in the store; and whether a time
    /// word is applied to a function's stream parameter, when every
    /// unrated stream keeps its ticks, since any may be passed there
    timed: std::collections::HashSet<String>,
    timed_all: bool,
    /// the streams the store keeps history in (log 89, question 47):
    /// the names a history word or a time word is applied to anywhere,
    /// and whether one is applied to a function's stream parameter, in
    /// which case every stream keeps its history, since any may be
    /// passed there. Every other stream is a queue: it holds an item
    /// only until its reader has passed it, and the slot comes back
    kept: std::collections::HashSet<String>,
    kept_all: bool,
    /// ... so nothing in the store is kept and every stream is a
    /// queue, and every read takes the queue's word, which needs no
    /// residency check. Where anything is kept, every read takes the
    /// ring's word, which is right on a queue too
    all_queues: bool,
    /// the streams made as queues, feature-scope and, per body, local:
    /// a push into one by name is `push_queue`
    queues: std::collections::HashSet<String>,
    queue_locals: std::collections::HashSet<String>,
    /// the feature-scope streams a function reads items of by name
    /// (question 48): a queue frees its slots only where the one
    /// reader is a node the front end emits, so a stream named here,
    /// or read by two nodes, keeps everything it is given
    /// does any wiring in the store carry `at (n hz)` (log 83)? If none
    /// does, every `__hz` is 0 and a task's pushes need no sleep
    any_rated_wiring: bool,
    /// the streams declared at a rate, feature-scope and, per body, local
    /// (log 57): a push into one calls the regular push directly, since
    /// the compiler chose the ring, where `__push` charges the clock's
    /// branch to every push in the cost model
    regular: std::collections::HashSet<String>,
    regular_locals: std::collections::HashSet<String>,
    /// the feature whose code is being lowered
    cur: String,
    /// every feature's layer height (log 28)
    ranks: HashMap<String, usize>,
    /// the features, in composition order
    features: Vec<String>,
    /// each feature's parent (log 51): the effective state of a feature
    /// is its own flag and every ancestor's, read in line by `gate`
    parents: HashMap<String, Option<String>>,
    /// the features the product marks static on (log 71): no field, no
    /// switch, no gate, and a chain body under its link's name
    statics: std::collections::HashSet<String>,
    /// the feature-scope streams that have a rate (log 77): declared
    /// with one, or wired to a rated task; a consumer takes each of
    /// their items at its tick
    rated: std::collections::HashSet<String>,
    /// ... and the hz of those declared with one, whose ring is regular
    /// at that rate (item k at tick k)
    rates: HashMap<String, i64>,
    /// the edge functions (log 72) by IR name, each with the stream it reads
    edge_fns: HashMap<String, String>,
    /// the feature-scope streams with no storage (question 50, fm3 log
    /// 92): only pushed into and wired by edges, no word anywhere in the
    /// store reading them. A push into one is the call of each edge out
    /// of it, and nothing is kept
    bare: std::collections::HashSet<String>,
    /// ... and the edges out of each, in composition order: the function
    /// of one item the front end wrote, and the feature whose gate it is
    /// called under
    bare_edges: HashMap<String, Vec<(String, String)>>,
    /// while a push statement into a bare stream is lowered: the stream,
    /// and each edge's gate, read once before the items (None where the
    /// edge's feature is static)
    bare_gates: Option<(String, Vec<Option<String>>)>,
    /// while a push statement is lowered: an item went by a `<<` method
    /// or a task call, which push inside themselves, so the statement
    /// keeps its trailing trigger though the stream is paced (log 93)
    loose_push: bool,
    /// as a statement is lowered: the stream the statement before it in
    /// the same block pushed into, if that was a plain push. A push into
    /// a stream with a rate leaves now at the end of a slot, so the next
    /// statement's push into the same stream is on the beat already (fm3
    /// log 98)
    after_push: Option<String>,
    /// the clock of code (question 56, fm3 log 99), settled by `Beat`
    /// before any function is lowered, each by the statement's address:
    /// the push statements the clock is known to be on the beat at,
    /// which need no alignment; the loops whose first statement's
    /// alignment is made once before them, with the stream's rate; and,
    /// as such a loop is lowered, that rate
    on_beat: std::collections::HashSet<usize>,
    loop_beats: HashMap<usize, i64>,
    loop_beat: Option<i64>,
    /// the product's clock (log 77)
    clock: super::store::Clock,
    /// the scheduler is a static schedule (log 78): the node graph is
    /// acyclic, and a push into a stream runs `__run_<stream>()`
    static_schedule: bool,
    /// the nodes their pushers wake (fm3 log 103), by the one stream
    /// each reads, in the schedule's order: a push into the stream or
    /// its first `end` calls the node's task in line, and the node
    /// keeps no `seen`, no `fin` and no `__node<k>` of its own
    wakes: HashMap<String, Vec<usize>>,
    /// ... and the nodes each node input reaches that are not woken,
    /// which the scheduler still runs after a push into it
    rests: HashMap<String, Vec<usize>>,
    /// can a trigger be met while a node runs? Where not, the
    /// scheduler has no `__running` and a trigger that reaches one node
    /// is that node's call
    guard: bool,
    /// while a push statement is lowered: its stream and its depth, so
    /// that a single item pushed at that depth is known to have arrived
    push_site: Option<(String, usize)>,
    /// ... and that one has
    sure_push: bool,
    /// the arrival bound (log 79): per feature-scope stream, the most
    /// items one push statement from a plain function or the reset
    /// pushes into it, None when some statement's count is unknown
    arrivals: HashMap<String, Option<i64>>,
    read_by_name: std::collections::HashSet<String>,
    /// the element type of every `end`'s operand, as each is lowered,
    /// and of every queue's push (fm3 log 108): a push asks whether
    /// its stream has ended only where an `end` could have reached a
    /// stream of its type, which is known once every body is lowered
    ended: Vec<Ty>,
    queue_pushes: std::collections::BTreeMap<String, Ty>,
    /// the fields of the context some function of the store writes (fm3
    /// log 110): every store the lowering emitted, a trial's and a
    /// pruned function's among them. A field outside it holds one
    /// value from the start of a case to its end
    written: std::collections::HashSet<String>,
    /// the woken nodes that keep their position and not a whole reader
    /// (fm3 log 111): those whose task gives back its parameter's own
    /// ring and rules, `ring_kept`
    placed: std::collections::HashSet<usize>,
    /// how many nodes read each feature-scope stream, counted before
    /// the nodes are emitted (question 48)
    node_reads: HashMap<String, usize>,
    /// the feature that declared each type
    type_feature: HashMap<String, String>,
    /// while `choose` tries a method: which round of literal typing
    /// this is (log 47); `Any` outside a trial
    round: Round,
    /// while a width the policy may take is tried (log 52): what an
    /// integer and a decimal literal are taken as
    trial: (Ty, Ty),
    /// inside a push chain's `while` (log 39): the candidate, which `_`
    /// reads as; None elsewhere, where `_` is a reduction's accumulator
    candidate: Option<Val>,
    /// the product's bounds (log 41), by function key
    product: HashMap<String, i64>,
    /// while the device copy of a `<<` method is lowered (log 87): the
    /// name of its stream parameter, which is the device rather than a
    /// stream, so that every push into it is the platform's write and a
    /// `<<` on it calls the device copy of that method
    device_param: Option<String>,
    /// the device copies emitted, by the IR name of the stream copy
    device_fns: HashMap<String, String>,
}

/// where a range's values go (log 41): a new ring with them resident,
/// or straight into an existing stream, pushed as a block
#[derive(Clone, Copy)]
enum RangeSink<'a> {
    New(Option<&'a str>),
    Into(&'a str, &'a Val),
}

/// what kind of body is being lowered: the scheduler runs after a push
/// in a plain function only, and a task sleeps after a push into its
/// own output (log 25)
#[derive(Clone, PartialEq)]
enum BodyKind {
    Fn,
    /// the output stream's name (none for a sink, log 57), and the IR
    /// name of the rate parameter
    Task { out: Option<String>, hz: String },
    Reset,
    Node,
}

/// on the right of `<<`, and in the chain's `while`, a stream's name
/// is its latest item (log 23, 39)
#[derive(Clone)]
enum PushRead {
    Latest(Val, Ty),
}

/// a variable in a function's scope: its current IR value and type
#[derive(Clone, Debug)]
struct Var {
    ir: String,
    ty: Ty,
    /// has it a value yet? A result starts without one
    set: bool,
    /// how many loops enclosed its declaration: a variable may be
    /// assigned only at the depth it was declared, or by the loop that
    /// carries it (log 12)
    loop_depth: usize,
}

/// a loop being lowered: what `break` and `continue` need
struct LoopCtx {
    /// the carried variables, in the header's order, then the streams
    /// the body moves (log 23); empty for a `for`
    carried: Vec<String>,
    /// what every `break` yields: the given variables (log 40), then
    /// the moved streams; empty for a `for`
    results: Vec<String>,
    /// how many of them the header declares: `continue (...)` gives
    /// those, and a carried stream takes its current reader
    explicit: usize,
    /// a `for`'s stepped variable: its name and the step (`add`/`sub`,
    /// the amount) — the item itself over a range, the index over a
    /// sequence
    item: Option<(String, &'static str, String)>,
    /// over a sequence, the item loaded at the top of each pass
    loaded: Option<String>,
    /// how many `break`s the body has, the `while` test's included
    breaks: usize,
}

/// a value: the text that stands for it as an operand — a name, or a
/// literal where the IR takes one inline — and its type
#[derive(Clone, Debug)]
struct Val {
    text: String,
    ty: Ty,
    literal: bool,
}

/// one function body being lowered
struct Body {
    out: String,
    ntmp: usize,
    /// the variables in scope; a block's declarations leave with it
    vars: HashMap<String, Var>,
    /// how many IR definitions each name has had, for the whole body,
    /// so a name declared again after its block closed is still SSA
    defs: HashMap<String, usize>,
    results: Vec<(String, Ty)>,
    file: String,
    /// how deep in structured blocks the next line is
    depth: usize,
    loops: Vec<LoopCtx>,
    kind: BodyKind,
    /// the function this body defines, and the link below it in its
    /// chain, which is what `existing` calls (log 28)
    func: Option<FnInfo>,
    below: Option<String>,
    /// the product's trip count for this function's loops (log 41):
    /// `bound <function>: N` in the store's `product.md`
    product_bound: Option<i64>,
}

impl Body {
    fn tmp(&mut self) -> String {
        self.ntmp += 1;
        format!("_{}", self.ntmp)
    }

    /// a loop's opening line, `loop(hdr) {`, carrying the product's
    /// bound when the function has one and the loop does not already
    /// show its count (a range with literal bounds does)
    fn open_loop(&mut self, prefix: &str, hdr: &str, counted: bool) {
        let bound = match self.product_bound {
            Some(n) if !counted => format!(" bound {}", n),
            _ => String::new(),
        };
        self.line(&format!("{}loop({}){}", prefix, hdr, bound));
    }

    fn line(&mut self, s: &str) {
        for _ in 0..=self.depth {
            self.out.push_str("    ");
        }
        self.out.push_str(s);
        self.out.push('\n');
    }

    /// a variable brought into scope, with no value yet
    fn declare(&mut self, name: &str, ty: Ty) {
        let depth = self.loops.len();
        self.vars.insert(name.to_string(), Var { ir: String::new(), ty, set: false, loop_depth: depth });
    }

    /// the IR name for a new definition of `name`: the name itself the
    /// first time, `name_2`, `name_3` after
    fn define(&mut self, name: &str, ty: Ty) -> String {
        let depth = self.loops.len();
        let n = self.defs.entry(name.to_string()).or_insert(0);
        *n += 1;
        let ir = if *n == 1 { name.to_string() } else { format!("{}_{}", name, n) };
        let v = self.vars.entry(name.to_string()).or_insert(Var { ir: String::new(), ty: ty.clone(), set: false, loop_depth: depth });
        v.ty = ty;
        v.set = true;
        v.ir = ir.clone();
        ir
    }

    /// the current IR values of some variables, in order
    fn current(&self, names: &[String]) -> Vec<String> {
        names.iter().map(|n| self.vars[n].ir.clone()).collect()
    }

    /// may `name` be assigned here? Only at the depth it was declared,
    /// which includes a loop's own carried variables; a `for`'s item
    /// and a variable of an enclosing block are not (log 12)
    fn assignable(&self, name: &str, line: usize) -> Result<(), Error> {
        let v = &self.vars[name];
        if let Some(l) = self.loops.last() {
            if l.item.as_ref().map(|(i, _, _)| i.as_str()) == Some(name) || l.loaded.as_deref() == Some(name) {
                return Err(lex::error(&self.file, line, format!("'{}' is the item of the `for`: it steps by itself and is not assigned", name)));
            }
        }
        if v.loop_depth != self.loops.len() {
            return Err(lex::error(&self.file, line, format!("'{}' is declared outside the loop: carry it in the loop's header, `loop ({} {} = ...)`", name, v.ty.ir(), name)));
        }
        Ok(())
    }

    /// a literal made into a value, where the IR wants a name
    fn materialize(&mut self, v: &Val) -> Val {
        if !v.literal {
            return v.clone();
        }
        let t = self.tmp();
        self.line(&format!("{}: {} = const {}", t, v.ty.ir(), v.text));
        Val { text: t, ty: v.ty.clone(), literal: false }
    }
}

impl Lowerer {
    /// may the feature being lowered name something of `owner`'s? Its
    /// own layer or a lower one, never up (log 28); the front end's
    /// builtins belong to no feature
    fn reach(&self, what: &str, owner: &str, file: &str, line: usize) -> Result<(), Error> {
        if owner.is_empty() || owner == self.cur {
            return Ok(());
        }
        let (from, to) = (self.ranks.get(&self.cur).copied().unwrap_or(0), self.ranks.get(owner).copied().unwrap_or(0));
        if to > from {
            return Err(lex::error(file, line, format!("'{}' belongs to feature {}, whose layer is above {}'s: a name reaches down or sideways, never up", what, owner, self.cur)));
        }
        Ok(())
    }

    /// a type by zero's name; with `seq`, a stream of it (`T x$`)
    fn ty(&mut self, name: &str, seq: bool, file: &str, line: usize) -> Result<Ty, Error> {
        let t = match builtin_type(name) {
            Some(t) => t,
            None => match self.types.get(name) {
                Some(TypeInfo::Struct(_)) => Ty::Struct(name.to_string()),
                Some(TypeInfo::Enum(_)) => Ty::Enum(name.to_string()),
                None => return Err(lex::error(file, line, format!("'{}' is not a type", name))),
            },
        };
        if let Some(owner) = self.type_feature.get(name) {
            self.reach(name, &owner.clone(), file, line)?;
        }
        if !seq {
            return Ok(t);
        }
        self.stream_ty(t, file, line)
    }

    /// a new stream whose items are all present (log 38): a ring of at
    /// least `cap` items — the count, or `RING_ITEMS` when that is
    /// larger — in the arena, stamped once at the clock's now; the
    /// caller pushes the items with `push s, t, x`
    fn new_resident(&mut self, elem: &Ty, cap: &str, b: &mut Body, dst: Option<&str>) -> (Val, Option<String>) {
        let ty = Ty::Stream(Box::new(elem.clone()));
        let maker = self.flavour(dst, b);
        self.rings.insert((elem.ir(), maker.to_string()));
        let cap = match cap.parse::<usize>() {
            Ok(n) => n.max(RING_ITEMS).to_string(),
            Err(_) => {
                let least = b.tmp();
                b.line(&format!("{}: i64 = const {}", least, RING_ITEMS));
                let m = b.tmp();
                b.line(&format!("{}: i64 = max({}, {})", m, cap, least));
                m
            }
        };
        let out = name_for(dst, &ty, b);
        b.line(&format!("{}: {} = __{}_{}({}, {})", out, ty.ir(), maker, elem.ir(), CLOCK_HZ, cap));
        if maker != "stream" {
            return (Val { text: out, ty, literal: false }, None);
        }
        let t = b.tmp();
        b.line(&format!("{}: i64 = __now()", t));
        (Val { text: out, ty, literal: false }, Some(t))
    }

    /// a view's items as a new stream, through the generated `__copy_T`
    /// a string literal's bytes as a view of `data`, and their count
    fn str_view(&mut self, s: &str, b: &mut Body) -> (String, String) {
        let (p, n) = self.str_data(s, b);
        let v = b.tmp();
        b.line(&format!("{}: u8[] = __str({}, {})", v, p, n));
        (v, n)
    }

    /// a string literal's bytes where they lie: the address of its
    /// `data` and its length
    fn str_data(&mut self, s: &str, b: &mut Body) -> (String, String) {
        let p = b.tmp();
        let n = b.tmp();
        if s.is_empty() {
            // `""` has no bytes to keep: the IR refuses an empty
            // `data`, so it is `__nul` with a length of zero
            b.line(&format!("{}: ptr = addr __nul", p));
            b.line(&format!("{}: i64 = const 0", n));
        } else {
            self.nstr += 1;
            let name = format!("__s{}", self.nstr);
            self.data.push(format!("data {} = \"{}\"", name, s.replace('\\', "\\\\").replace('"', "\\\"").replace('\n', "\\n")));
            b.line(&format!("{}: ptr = addr {}", p, name));
            b.line(&format!("{}: i64 = len {}", n, name));
        }
        (p, n)
    }

    fn copy_view(&mut self, elem: &Ty, view: &str, b: &mut Body, dst: Option<&str>) -> Val {
        let ty = Ty::Stream(Box::new(elem.clone()));
        let maker = self.flavour(dst, b);
        self.copies.insert((elem.ir(), maker.to_string()));
        self.rings.insert((elem.ir(), maker.to_string()));
        let out = name_for(dst, &ty, b);
        b.line(&format!("{}: {} = __copy_{}{}({})", out, ty.ir(), copy_infix(maker), elem.ir(), view));
        Val { text: out, ty, literal: false }
    }

    /// What storage does a stream get (log 73, 89, questions 42, 47)?
    /// The words in the store's text decide, and nothing else. Where
    /// something asks a time of it — a time word on its name, or one
    /// on any stream parameter — its ring keeps a tick per item
    /// (`stream`). Where a history word keeps it but nothing times it,
    /// it is a ring of values with a position (`regular`). Where
    /// neither, it is a `queue`: it holds an item only until its
    /// reader has passed it, and the slot comes back
    fn flavour(&mut self, name: Option<&str>, b: &Body) -> &'static str {
        // a queue's slots are a plain run, which the ring's words would
        // read wrong, and a read through a function's stream parameter
        // cannot be told apart — so a store that keeps anything keeps
        // everything, and only an all-queue store has queues (log 89)
        let kept = if !self.all_queues {
            true
        } else {
            match name {
                Some(n) => self.kept.contains(n),
                None => false,
            }
        };
        let word = if !kept {
            "queue"
        } else if self.timed_all || name.is_some_and(|n| self.timed.contains(n)) {
            "stream"
        } else {
            "regular"
        };
        if let Some(n) = name {
            if word != "stream" {
                let set = if b.kind == BodyKind::Reset { &mut self.regular } else { &mut self.regular_locals };
                set.insert(n.to_string());
            }
            if word == "queue" {
                let set = if b.kind == BodyKind::Reset { &mut self.queues } else { &mut self.queue_locals };
                set.insert(n.to_string());
            }
        }
        word
    }

    /// May a node give a queue's slots back when it has run (log 89,
    /// question 48)? The stream must be a queue, this node must be its
    /// only node, and no function in the store may read its items by
    /// name: `consumed` is one position, and several readers would
    /// want the least of them, which no reader can compute alone
    fn frees(&self, sname: &str) -> bool {
        if !self.queues.contains(sname) || self.read_by_name.contains(sname) {
            return false;
        }
        self.node_reads.get(sname) == Some(&1)
    }

    /// The word of a queue's push, written with the stream's element
    /// type beside it and settled by `settle_pushes` when every body
    /// has been lowered (fm3 log 108), since an `end` that reaches the
    /// type may be lowered after the push. A stream whose type is not
    /// in hand keeps the checked push
    fn queue_push(&mut self, elem: Option<&Ty>) -> String {
        let Some(e) = elem else { return "push_queue".to_string() };
        self.queue_pushes.insert(e.mangled(), e.clone());
        format!("push_queue<{}>", e.mangled())
    }

    /// Is a push into this name the queue's (log 89)? Its ring was
    /// made as a queue where the front end could see it, and in a
    /// store nothing keeps history in every stream is one
    fn is_queue(&self, name: &str, b: &Body) -> bool {
        if self.all_queues {
            return true;
        }
        if b.vars.contains_key(name) { self.queue_locals.contains(name) } else { self.queues.contains(name) }
    }

    /// a stream's unread items as one view, the reader not moved: what
    /// the sequence words read (log 38)
    fn unread_view(&mut self, s: &Val, b: &mut Body) -> String {
        let e = s.ty.elem().unwrap();
        let v = b.tmp();
        let word = if self.all_queues { "unread_queue" } else { "unread" };
        b.line(&format!("{}: {}[] = {}({})", v, e.ir(), word, s.text));
        v
    }

    /// how many items a stream has unread, as an int
    fn count_of(&mut self, s: &Val, b: &mut Body, dst: Option<&str>) -> Val {
        let first = s.text.clone();
        let n = b.tmp();
        b.line(&format!("{}: i64 = count {}", n, first));
        let ty = Ty::Num("int".into());
        let out = name_for(dst, &ty, b);
        b.line(&format!("{}: int = conv {}", out, n));
        Val { text: out, ty, literal: false }
    }

    /// the i-th unread item of a stream: `x$[i]`, `peek x$ at (i)`
    fn peek_at(&mut self, s: &Val, i: &str, b: &mut Body, dst: Option<&str>) -> Val {
        let elem = s.ty.elem().unwrap().clone();
        let out = name_for(dst, &elem, b);
        if self.all_queues {
            // a queue's reader needs no residency check: the push has
            // already proved that nothing unread was overwritten (log 89)
            b.line(&format!("{}: {} = peek_queue({}, {})", out, elem.ir(), s.text, i));
        } else {
            b.line(&format!("{}: {} = peek {}, {}", out, elem.ir(), s.text, i));
        }
        Val { text: out, ty: elem, literal: false }
    }

    fn declare_type(&mut self, t: &super::syntax::TypeDecl, file: &str) -> Result<(), Error> {
        if self.types.contains_key(&t.name) || builtin_type(&t.name).is_some() {
            return Err(lex::error(file, t.line, format!("type '{}' is already declared", t.name)));
        }
        match &t.kind {
            TypeKind::Enum(cases) => {
                // a byte, or the next memory width, so that a variable
                // of the type can be a field of the context (log 18)
                let bits = if cases.len() <= 256 { 8 } else if cases.len() <= 65536 { 16 } else { 32 };
                self.type_lines.push(format!("type {} = u{}", t.name, bits));
                self.types.insert(t.name.clone(), TypeInfo::Enum(cases.clone()));
                self.type_feature.insert(t.name.clone(), self.cur.clone());
            }
            TypeKind::Struct(fields) => {
                let mut out = Vec::new();
                let mut ir = Vec::new();
                for f in fields {
                    let ty = self.ty(&f.ty, f.seq, file, f.line)?;
                    if out.iter().any(|(n, _, _)| n == &f.name) {
                        return Err(lex::error(file, f.line, format!("field '{}' is named twice", f.name)));
                    }
                    let default = match &f.default {
                        None => None,
                        Some(Expr { kind: ExprKind::Int(v), .. }) if matches!(ty, Ty::Num(_)) => Some(v.to_string()),
                        Some(Expr { kind: ExprKind::Float(s), .. }) if matches!(ty, Ty::Num(_)) => Some(s.clone()),
                        Some(Expr { kind: ExprKind::Bool(v), .. }) if ty == Ty::Bool => Some((*v as i64).to_string()),
                        Some(e) => return Err(lex::error(file, e.line, format!("a field's default is a literal of its type ({})", ty.ir()))),
                    };
                    ir.push(format!("{}: {}", f.name, ty.ir()));
                    out.push((f.name.clone(), ty, default));
                }
                if out.is_empty() {
                    return Err(lex::error(file, t.line, format!("type {} has no fields", t.name)));
                }
                self.type_lines.push(format!("type {} = struct\n    {}", t.name, ir.join("\n    ")));
                self.types.insert(t.name.clone(), TypeInfo::Struct(out));
                self.type_feature.insert(t.name.clone(), self.cur.clone());
            }
        }
        Ok(())
    }

    fn declare(&mut self, f: &FnDecl, feature: &str, file: &str) -> Result<(), Error> {
        // a platform function (section 15, log 31): its bodies are its
        // platform bodies, one per kind of place, and nothing else
        let platform = if f.platform.is_empty() {
            None
        } else {
            if !f.body.is_empty() {
                return Err(lex::error(file, f.line, "a platform function has no zero body: its platform bodies are its only ones"));
            }
            if f.task {
                return Err(lex::error(file, f.line, "a task is not a platform function"));
            }
            let mut kinds: Vec<String> = Vec::new();
            for (ks, lines) in &f.platform {
                if lines.is_empty() {
                    return Err(lex::error(file, f.line, format!("the platform body for {} has no lines", ks.join(" "))));
                }
                for k in ks {
                    if !KINDS.contains(&k.as_str()) {
                        return Err(lex::error(file, f.line, format!("'{}' is not a kind of place here: one of {}", k, KINDS.join(", "))));
                    }
                    if kinds.contains(k) {
                        return Err(lex::error(file, f.line, format!("two platform bodies for {}", k)));
                    }
                    kinds.push(k.clone());
                }
            }
            Some(kinds)
        };
        let key = mangle(&f.name);
        let mut params = Vec::new();
        for p in f.params() {
            // a `$` parameter is a stream, which a task reads and moves (log 25)
            params.push((p.name.clone(), self.ty(&p.ty, p.seq, file, p.line)?));
        }
        let mut results = Vec::new();
        if f.task {
            let [r] = f.results.as_slice() else {
                return Err(lex::error(file, f.line, "a task produces one stream: `on (T x$) << name (...)`"));
            };
            if !r.seq {
                return Err(lex::error(file, f.line, format!("a task's result is the stream it produces: `on ({} {}$) << ...`", r.ty, r.name)));
            }
        }
        for r in &f.results {
            results.push((r.name.clone(), self.ty(&r.ty, r.seq, file, r.line)?));
        }
        let operator = f.name.iter().any(|p| matches!(p, NamePart::Sym(_)));
        if operator && f.task {
            return Err(lex::error(file, f.line, "a task has a name, not a symbol"));
        }
        if let Some(kinds) = &platform {
            // a rule takes the machine's own types: one number each way
            if kinds.iter().any(|k| k != "ir") {
                for (n, t) in params.iter().chain(&results) {
                    let concrete = matches!(t, Ty::Num(x) if x.len() > 1 && x[1..].parse::<u32>().is_ok());
                    if !concrete {
                        return Err(lex::error(file, f.line, format!("'{}' is {} in a platform rule: a rule takes the machine's types, int64 and the like, one number each way", n, t.ir())));
                    }
                }
                if results.len() > 1 {
                    return Err(lex::error(file, f.line, "a platform rule gives one result at most"));
                }
            }
            if operator {
                return Err(lex::error(file, f.line, "an operator is not a platform function"));
            }
        }
        let push_method = matches!(f.name.as_slice(), [NamePart::Group, NamePart::Sym(s), NamePart::Group] if s == "<<");
        if operator && push_method {
            // a `<<` method (log 59): a stream, then the item's type, no result
            if params.len() != 2 || !matches!(params[0].1, Ty::Stream(_)) {
                return Err(lex::error(file, f.line, "a `<<` method is `on (T o$) << (U x)`: a stream, then what is pushed into it"));
            }
            if !results.is_empty() {
                return Err(lex::error(file, f.line, "a `<<` method has no result: a push moves no reader"));
            }
            let elem = params[0].1.elem().cloned().unwrap();
            if params[1].1 == elem {
                return Err(lex::error(file, f.line, format!("'{}' pushed into '{}' is the push itself, not a method", zero_ty(&params[1].1), zero_ty(&params[0].1))));
            }
            if matches!(params[1].1, Ty::Stream(ref e) if **e == elem) {
                return Err(lex::error(file, f.line, format!("'{}' pushed into '{}' is the block push of section 9, not a method", zero_ty(&params[1].1), zero_ty(&params[0].1))));
            }
        } else if operator {
            // an operator on a declared type: the IR does not dispatch
            // arithmetic on structs, so it is a function named by the
            // opcode and its first operand's type (log 9)
            if f.name.len() != 3 || !matches!(f.name.as_slice(), [NamePart::Group, NamePart::Sym(_), NamePart::Group]) || params.len() != 2 {
                return Err(lex::error(file, f.line, "an operator is `on (T r) = (T a) op (U b)`"));
            }
            if !matches!(params[0].1, Ty::Struct(_)) {
                return Err(lex::error(file, f.line, "an operator's first operand is a declared struct type; numbers have the IR's operators"));
            }
        }
        // a name is a set of methods (section 6, log 36): every function
        // with these words. The same parameter types are the same
        // method, which a later feature redefines and chains (log 28);
        // other types are a new method of the name, told apart in the
        // IR by its types
        let set: Vec<usize> = self.funcs.iter().enumerate().filter(|(_, g)| g.key == key).map(|(i, _)| i).collect();
        if let Some(&i) = set.iter().find(|&&i| self.funcs[i].parts != f.name) {
            return Err(lex::error(file, f.line, format!("'{}' clashes with a function of feature {} that mangles to the same name", key, self.funcs[i].feature)));
        }
        let ptys = |g: &FnInfo| g.params.iter().map(|(_, t)| t.clone()).collect::<Vec<Ty>>();
        let rtys = |g: &FnInfo| g.results.iter().map(|(_, t)| t.clone()).collect::<Vec<Ty>>();
        let mine = FnInfo { key: key.clone(), ir: String::new(), plain: String::new(), in_set: false, parts: f.name.clone(), params, results, feature: feature.to_string(), task: f.task, chain: vec![feature.to_string()], platform };
        if let Some(&i) = set.iter().find(|&&i| ptys(&self.funcs[i]) == ptys(&mine)) {
            let other = &self.funcs[i];
            if other.platform.is_some() || mine.platform.is_some() {
                return Err(lex::error(file, f.line, format!("'{}' is a platform function of feature {}: the platform is called, not redefined (section 15)", spelled(other), other.feature)));
            }
            let last = other.chain.last().cloned().unwrap_or_default();
            if last == feature {
                return Err(lex::error(file, f.line, format!("'{}' is defined twice in feature {}", spelled(other), feature)));
            }
            if other.task || f.task {
                return Err(lex::error(file, f.line, format!("'{}' is a task: a task is not redefined in this milestone", spelled(other))));
            }
            // a `<<` method is redefined and chained like any other
            // function: it is how a feature watches what a program
            // writes, the device being written and never read
            // (question 46, log 90). Arithmetic on a declared type is
            // still not redefined in this milestone
            if operator && !push_method {
                return Err(lex::error(file, f.line, "an operator is not redefined in this milestone; a `<<` method is"));
            }
            if rtys(other) != rtys(&mine) {
                return Err(lex::error(file, f.line, format!("'{}' redefines feature {}'s with different results: a redefinition keeps the signature", spelled(other), last)));
            }
            self.reach(&spelled(other), &last, file, f.line)?;
            // the method stays the first definer's: a lower layer's
            // name, which control may flow up through (section 12)
            self.funcs[i].chain.push(feature.to_string());
            return Ok(());
        }
        if let Some(&i) = set.first() {
            if self.funcs[i].task || f.task {
                return Err(lex::error(file, f.line, format!("'{}' is a task: a task's name has one method", spoken(&self.funcs[i]))));
            }
        }
        // an operator on a declared type is named now, `add_Vec`, and
        // never forms a set, the IR's `add` being an operation; the
        // other methods are named once every feature has declared
        // (`name_methods`, log 52)
        let (ir, plain) = if operator {
            let tys: Vec<String> = mine.params.iter().map(|(_, t)| t.mangled()).collect();
            let op = if set.is_empty() && !push_method { format!("{}_{}", key, mine.params[0].1.ir()) } else { crate::ssa::method_name(&key, &tys) };
            (op.clone(), op)
        } else {
            (String::new(), String::new())
        };
        self.funcs.push(FnInfo { ir, plain, ..mine });
        Ok(())
    }

    /// The IR names of a name's methods (log 36, 52): the first keeps
    /// the key, a later one is the key and its parameter types,
    /// `describe__float`, as `ssa::method_name` spells it. A name all of
    /// whose methods are concrete is emitted as one IR method set —
    /// every definition under the key, the IR naming the later ones
    /// the same way — so a call on the key with a literal typed `int`
    /// or `float` is resolved by the policy. A name with a method over
    /// an abstract type is not: such a method is a template in the IR,
    /// instantiated under names of its own, so each method keeps its
    /// unique name and the first, a lone template, its default
    /// instance under the key
    fn name_methods(&mut self, files: &HashMap<String, String>) -> Result<(), Error> {
        let mut keys: Vec<String> = Vec::new();
        for f in &self.funcs {
            if f.ir.is_empty() && !keys.contains(&f.key) {
                keys.push(f.key.clone());
            }
        }
        for key in keys {
            let idx: Vec<usize> = (0..self.funcs.len()).filter(|&i| self.funcs[i].key == key && self.funcs[i].ir.is_empty()).collect();
            let set = idx.len() > 1 && idx.iter().all(|&i| self.funcs[i].params.iter().all(|(_, t)| is_concrete(t)));
            for (k, &i) in idx.iter().enumerate() {
                let tys: Vec<String> = self.funcs[i].params.iter().map(|(_, t)| t.ir()).collect();
                let ir = if k == 0 { key.clone() } else { crate::ssa::method_name(&key, &tys) };
                self.funcs[i].plain = if set { key.clone() } else { ir.clone() };
                self.funcs[i].ir = ir;
                self.funcs[i].in_set = set;
            }
        }
        for i in 0..self.funcs.len() {
            if let Some(j) = (0..i).find(|&j| self.funcs[j].ir == self.funcs[i].ir) {
                let (mine, other) = (&self.funcs[i], &self.funcs[j]);
                return Err(lex::error(files.get(&mine.feature).map(String::as_str).unwrap_or(""), 0, format!("'{}' would be {} in the IR, which feature {}'s '{}' already is", spelled(mine), mine.ir, other.feature, spelled(other))));
            }
        }
        Ok(())
    }

    /// the task a phrase calls, its arguments, and the rate after it —
    /// `count down from (10) at (1 hz)` — or None when the phrase is not
    /// a task call (the ordinary call path then reports what it is)
    fn task_call(&self, e: &Expr, vars: Option<&HashMap<String, Var>>, file: &str) -> Result<Option<(FnInfo, Vec<Expr>, i64)>, Error> {
        let ExprKind::Phrase(parts) = &e.kind else { return Ok(None) };
        let (parts, hz) = match parts.as_slice() {
            [rest @ .., Part::Word(at), Part::Args(a)] if at == "at" && a.len() == 1 && matches!(&a[0].value.kind, ExprKind::Unit(..)) => (rest, Some(&a[0].value)),
            _ => (parts.as_slice(), None),
        };
        let is_var = |w: &str| vars.is_some_and(|v| v.contains_key(w)) || self.fvar(w).is_some();
        let Ok((cands, args)) = find_methods(&self.funcs, parts, &is_var, file, e.line) else {
            return Ok(None);
        };
        // a task's name has one method (log 36)
        let info = cands[0];
        if !info.task {
            return Ok(None);
        }
        let hz = match hz {
            Some(r) => self.rate_hz(r, file)?,
            None => 0,
        };
        Ok(Some((info.clone(), args, hz)))
    }

    /// the stream variables a task call moves: its arguments at the
    /// task's stream parameters
    fn task_streams(&self, parts: &[Part]) -> Vec<String> {
        let e = Expr { kind: ExprKind::Phrase(parts.to_vec()), line: 0 };
        let Ok(Some((info, args, _))) = self.task_call(&e, None, "") else { return Vec::new() };
        args.iter()
            .zip(&info.params)
            .filter_map(|(a, (_, t))| match (&a.kind, t) {
                (ExprKind::Seq(n), Ty::Stream(_)) => Some(n.clone()),
                _ => None,
            })
            .collect()
    }

    /// Run a task now (log 25): into the stream `out`, with `__hz` the
    /// rate given, the enclosing task's, or none; each stream argument
    /// is a stream variable, and takes the reader the task returns
    fn run_task(&mut self, info: &FnInfo, args: &[Expr], hz: i64, out: &Val, b: &mut Body, line: usize) -> Result<(), Error> {
        let file = b.file.clone();
        self.reach(&spoken(info), &info.feature, &file, line)?;
        if info.results.is_empty() {
            return Err(lex::error(&file, line, format!("'{}' is a sink: it is wired at feature scope, `{}(...)`, and not run here", spoken(info), info.key)));
        }
        let Ty::Stream(want) = &info.results[0].1 else { unreachable!() };
        let Ty::Stream(have) = &out.ty else { unreachable!() };
        if want != have {
            return Err(lex::error(&file, line, format!("'{}' produces {} and '{}' holds {}", info.key, want.ir(), out.text, have.ir())));
        }
        let mut ops = vec![out.text.clone()];
        let mut moved: Vec<(String, Ty)> = Vec::new();
        for (a, (pname, pty)) in args.iter().zip(&info.params) {
            if let Ty::Stream(pe) = pty {
                let ExprKind::Seq(n) = &a.kind else {
                    return Err(lex::error(&file, a.line, format!("'{}' reads '{}$' as a stream it moves: give it a stream variable", info.key, pname)));
                };
                let Some(Ty::Stream(ae)) = self.stream_var(n, b) else {
                    return Err(lex::error(&file, a.line, format!("'{}$' is not declared: '{}' reads a stream here", n, info.key)));
                };
                if ae != *pe {
                    return Err(lex::error(&file, a.line, format!("'{}' reads a stream of {}, '{}$' holds {}", info.key, pe.ir(), n, ae.ir())));
                }
                let v = self.lower_expr(&Expr { kind: ExprKind::Name(n.clone()), line: a.line }, None, b, None)?;
                if b.vars.contains_key(n) {
                    b.assignable(n, a.line)?;
                }
                ops.push(v.text);
                moved.push((n.clone(), v.ty));
            } else {
                ops.push(self.plain_arg(info, a, pty, b)?);
            }
        }
        ops.push(match (&b.kind, hz) {
            (BodyKind::Task { hz: h, .. }, 0) => h.clone(),
            _ => format!("{}: i64", hz),
        });
        // the moved readers: a local's next version, a feature variable's field stored
        let mut defs = Vec::new();
        let mut sets = Vec::new();
        for (n, ty) in &moved {
            if b.vars.contains_key(n) {
                let ir = b.define(n, ty.clone());
                defs.push(format!("{}: {}", ir, ty.ir()));
            } else {
                let t = b.tmp();
                sets.push((n.clone(), t.clone()));
                defs.push(format!("{}: {}", t, ty.ir()));
            }
        }
        let call = format!("{}({})", info.ir, ops.join(", "));
        if defs.is_empty() {
            b.line(&call);
        } else {
            b.line(&format!("{} = {}", defs.join(", "), call));
        }
        for (n, t) in sets {
            self.field_put(&n, &t, b);
        }
        Ok(())
    }

    /// The nodes a feature-scope declaration wires (log 25): `T x$ =
    /// task(...)`, or task calls in a `<<` chain after its pushed items
    fn collect_nodes(&mut self, v: &super::syntax::VarDecl, feature: &str, file: &str) -> Result<(), Error> {
        let calls: Vec<&Expr> = match &v.init {
            Some(Init::Value(e)) if self.task_call(e, None, file)?.is_some() => vec![e],
            Some(Init::Pushes { items, cond, .. }) => {
                let mut first = None;
                for (i, e) in items.iter().enumerate() {
                    if self.task_call(e, None, file)?.is_some() {
                        first = Some(i);
                        break;
                    }
                }
                let Some(first) = first else { return Ok(()) };
                if cond.is_some() {
                    return Err(lex::error(file, v.line, "a task call is not repeated with `while`: the task's own chain says when it stops"));
                }
                items[first..].iter().collect()
            }
            _ => return Ok(()),
        };
        for e in calls {
            let Some((info, args, hz)) = self.task_call(e, None, file)? else {
                return Err(lex::error(file, e.line, "a value pushed after a task call at feature scope: a chain's items come before its tasks"));
            };
            if info.results.is_empty() {
                return Err(lex::error(file, e.line, format!("'{}' is a sink: it fills no stream, so it is wired alone, `{}`", spoken(&info), phrase_text(e))));
            }
            self.reach(&spoken(&info), &info.feature, file, e.line)?;
            for (a, (pname, pty)) in args.iter().zip(&info.params) {
                if let Ty::Stream(pe) = pty {
                    let ExprKind::Seq(n) = &a.kind else {
                        return Err(lex::error(file, a.line, format!("'{}' reads '{}$' as a stream: wire a feature-scope stream to it", info.key, pname)));
                    };
                    match self.fvar(n).map(|f| f.ty.clone()) {
                        Some(Ty::Stream(ae)) if ae == *pe => {}
                        Some(Ty::Stream(ae)) => return Err(lex::error(file, a.line, format!("'{}' reads a stream of {}, '{}$' holds {}", info.key, pe.ir(), n, ae.ir()))),
                        Some(t) => return Err(lex::error(file, a.line, format!("'{}$' is a {}, not a stream", n, t.ir()))),
                        None => return Err(lex::error(file, a.line, format!("'{}$' is not a feature-scope stream", n))),
                    }
                    self.reach(&format!("{}$", n), &self.fvar(n).unwrap().feature.clone(), file, a.line)?;
                    self.node_inputs.insert(n.clone());
                }
            }
            let text = format!("{} {}$ {} {}", v.ty, v.name, if matches!(v.init, Some(Init::Value(_))) { "=" } else { "<<" }, phrase_text(e));
            self.nodes.push(Node { info, out: Some(v.name.clone()), args, hz, feature: feature.to_string(), file: file.to_string(), text });
        }
        Ok(())
    }

    /// The sinks a store wires (log 57): a bare phrase at feature scope,
    /// `write(out$)`, names a function with a `$` parameter and no
    /// result, which is a task from that line on — in the IR its stream
    /// parameters are returned moved on, so the node carries its reader
    fn mark_sink(&mut self, e: &Expr, feature: &str, file: &str) -> Result<(), Error> {
        let ExprKind::Phrase(parts) = &e.kind else {
            return Err(lex::error(file, e.line, "a line at feature scope is a declaration, or a wiring `sink(x$)`"));
        };
        if let [.., Part::Word(at), Part::Args(a)] = parts.as_slice() {
            if at == "at" && a.len() == 1 && matches!(&a[0].value.kind, ExprKind::Unit(..)) {
                return Err(lex::error(file, e.line, "a sink has no rate: it runs when its input has more"));
            }
        }
        let is_var = |w: &str| self.fvar(w).is_some();
        let (cands, args) = find_methods(&self.funcs, parts, &is_var, file, e.line)?;
        let i = self.funcs.iter().position(|g| g.key == cands[0].key && g.parts == cands[0].parts).unwrap();
        let info = &self.funcs[i];
        if !info.results.is_empty() {
            return Err(lex::error(file, e.line, format!("'{}' gives a result: a wiring fills a stream, `{} x$ = {}`, or names a sink, a function with a `$` parameter and no result", spoken(info), if info.task { task_elem(info) } else { "T".into() }, phrase_text(e))));
        }
        if !info.params.iter().any(|(_, t)| matches!(t, Ty::Stream(_))) {
            return Err(lex::error(file, e.line, format!("'{}' reads no stream: a sink has a `$` parameter", spoken(info))));
        }
        if info.chain.len() > 1 {
            return Err(lex::error(file, e.line, format!("'{}' is a sink: a task is not redefined in this milestone", spoken(info))));
        }
        let _ = (args, feature);
        self.funcs[i].task = true;
        Ok(())
    }

    /// the node a wiring line declares: its inputs are feature-scope
    /// streams, and it fills nothing
    fn collect_wire(&mut self, e: &Expr, feature: &str, file: &str) -> Result<(), Error> {
        let Some((info, args, hz)) = self.task_call(e, None, file)? else { unreachable!() };
        self.reach(&spoken(&info), &info.feature, file, e.line)?;
        for (a, (pname, pty)) in args.iter().zip(&info.params) {
            if let Ty::Stream(pe) = pty {
                let ExprKind::Seq(n) = &a.kind else {
                    return Err(lex::error(file, a.line, format!("'{}' reads '{}$' as a stream: wire a feature-scope stream to it", info.key, pname)));
                };
                match self.fvar(n).map(|f| f.ty.clone()) {
                    Some(Ty::Stream(ae)) if ae == *pe => {}
                    Some(Ty::Stream(ae)) => return Err(lex::error(file, a.line, format!("'{}' reads a stream of {}, '{}$' holds {}", info.key, pe.ir(), n, ae.ir()))),
                    Some(t) => return Err(lex::error(file, a.line, format!("'{}$' is a {}, not a stream", n, t.ir()))),
                    None => return Err(lex::error(file, a.line, format!("'{}$' is not a feature-scope stream", n))),
                }
                self.reach(&format!("{}$", n), &self.fvar(n).unwrap().feature.clone(), file, a.line)?;
                self.node_inputs.insert(n.clone());
            }
        }
        let text = phrase_text(e);
        self.nodes.push(Node { info, out: None, args, hz, feature: feature.to_string(), file: file.to_string(), text });
        Ok(())
    }

    /// Which feature-scope streams have no storage (question 50, fm3 log
    /// 92). Storage depends on the words applied to a stream: a history
    /// or a time word makes a ring, a reading word a queue, and a stream
    /// no word reads has none. A stream is bare when it is the source of
    /// an edge, is declared with nothing after its name but perhaps a
    /// rate, is not the platform's, and nothing in the store names it
    /// except as the target of a push: no function, declaration, wiring
    /// or case, a name a function has bound itself being that
    /// function's. Two more are left their queues: a stream whose type
    /// a `<<` method takes, since such a push passes the stream as a
    /// value; and one that reaches itself through bare edges, whose
    /// functions would call each other for ever. A stream that is
    /// pushed into and is the source of no edge is bare on the same
    /// conditions (question 54, fm3 log 96), which is what a stream
    /// becomes when the feature that wires it is left out by the
    /// product; one that no feature of the store reads or wires at all
    /// is refused. And a stream that is declared and named nowhere
    /// else, not even pushed into, has no storage and is not refused
    /// (question 57, fm3 log 100)
    fn settle_bare(&mut self, store: &Store) -> Result<(), Error> {
        let call = |parts: &[Part], bound: &Names, file: &str| -> Option<Vec<Expr>> {
            let is_var = |w: &str| bound.contains(w) || self.fvar(w).is_some();
            find_methods(&self.funcs, parts, &is_var, file, 0).ok().map(|(_, args)| args)
        };
        let (named, wires, pushed) = stream_uses(&store.features, &|e, file| matches!(self.task_call(e, None, file), Ok(Some(_))), &call);
        // the features the product leaves out are asked one thing: does
        // any of them read or wire a stream (question 54). Their tasks
        // are not declared, and a task call only ever made the target of
        // its push count as named, which is no reading
        let (named_out, wires_out, _) = stream_uses(&store.left_out, &|_, _| false, &|_, _, _| None);
        let mut bare = Names::new();
        let mut rates: HashMap<String, i64> = HashMap::new();
        // the sources of the edges, then the streams that are pushed into
        // and neither read nor wired (question 54, fm3 log 96): such a
        // stream has no storage either, a push into it being nothing but
        // the step of its rate, so a feature marked `static off` and the
        // same feature switched off are one program
        let mut unwired: Vec<&String> = pushed.iter().filter(|s| !wires.iter().any(|(w, _)| &w == s)).collect();
        unwired.sort();
        let sources = wires.iter().map(|(s, _)| (s, true)).chain(unwired.into_iter().map(|s| (s, false)));
        for (s, wired) in sources {
            if named.contains(s) || bare.contains(s) {
                continue;
            }
            let Some((feat, v)) = store.features.iter().find_map(|g| g.code.decls.iter().find_map(|d| match d { Decl::Var(v) if &v.name == s => Some((g, v)), _ => None })) else { continue };
            if feat.name == "platform" || !v.seq || v.init.is_some() {
                continue;
            }
            let Some(ty) = self.fvar(s).map(|f| f.ty.clone()) else { continue };
            if !matches!(ty, Ty::Stream(_)) {
                continue;
            }
            if self.funcs.iter().any(|g| matches!(g.parts.as_slice(), [NamePart::Group, NamePart::Sym(op), NamePart::Group] if op == "<<") && g.params.first().map(|p| &p.1) == Some(&ty)) {
                continue;
            }
            // ... and where no feature of the store as it was read, the
            // ones the product leaves out included, reads or wires it,
            // that is almost certainly a mistyped name
            if !wired && !named_out.contains(s) && !wires_out.iter().any(|(w, _)| w == s) {
                return Err(lex::error(&feat.code.file, v.line, format!("'{}$' is pushed into and nothing reads it or wires it, in any feature of the store, compiled in or left out: a mistyped name?", s)));
            }
            if let Some(r) = &v.rate {
                rates.insert(s.clone(), self.rate_hz(r, &feat.code.file)?);
            }
            bare.insert(s.clone());
        }
        // ... and a stream the program names nowhere at all, not even as
        // the target of a push (question 57, fm3 log 100): no storage
        // either, and no refusal, since it may be declared ahead of the
        // feature that will use it. Nothing can pass it to a method
        for g in &store.features {
            for d in &g.code.decls {
                let Decl::Var(v) = d else { continue };
                if g.name == "platform" || !v.seq || v.init.is_some() || named.contains(&v.name) || pushed.contains(&v.name) || wires.iter().any(|(s, t)| s == &v.name || t == &v.name) {
                    continue;
                }
                if !self.fvar(&v.name).is_some_and(|f| matches!(f.ty, Ty::Stream(_))) {
                    continue;
                }
                if let Some(r) = &v.rate {
                    rates.insert(v.name.clone(), self.rate_hz(r, &g.code.file)?);
                }
                bare.insert(v.name.clone());
            }
        }
        // a stream that reaches itself through bare edges keeps its queue
        loop {
            let round = |from: &String| -> bool {
                let mut seen = Names::new();
                let mut work: Vec<&String> = wires.iter().filter(|(s, _)| s == from).map(|(_, t)| t).collect();
                while let Some(t) = work.pop() {
                    if t == from {
                        return true;
                    }
                    if bare.contains(t) && seen.insert(t.clone()) {
                        work.extend(wires.iter().filter(|(s, _)| s == t).map(|(_, t)| t));
                    }
                }
                false
            };
            let looped: Vec<String> = bare.iter().filter(|s| round(s)).cloned().collect();
            if looped.is_empty() {
                break;
            }
            for s in looped {
                bare.remove(&s);
            }
        }
        for (s, hz) in rates {
            if bare.contains(&s) {
                self.rates.insert(s, hz);
            }
        }
        self.bare = bare;
        Ok(())
    }

    /// is the name a stream with no storage, not shadowed here?
    fn is_bare(&self, name: &str, b: &Body) -> bool {
        !b.vars.contains_key(name) && self.bare.contains(name)
    }

    /// An edge (log 72, zero.md section 9): `out$ << i$ << "\n"` at
    /// feature scope wires `i$` into `out$`. It is a sink the front end
    /// writes for itself — a loop of `count`, `peek`, the pushes and
    /// `advance`, the lexer's shape — wired as `write(out$)` is (log
    /// 57), so the scheduler moves each item as it arrives, by the
    /// dispatch a push in a function uses, and pushes the rest of the
    /// chain after each item (question 38)
    fn collect_edge(&mut self, target: &Expr, items: &[Expr], cond: Option<&Expr>, line: usize, feature: &str, file: &str) -> Result<(), Error> {
        let ExprKind::Seq(tname) = &target.kind else {
            return Err(lex::error(file, line, "`<<` pushes into a stream, named `x$`"));
        };
        let Some(tf) = self.fvar(tname).cloned() else {
            return Err(lex::error(file, line, format!("'{}$' is not a feature-scope stream", tname)));
        };
        let Ty::Stream(telem) = &tf.ty else {
            return Err(lex::error(file, line, format!("'{}$' is a {}, not a stream", tname, tf.ty.ir())));
        };
        if self.input_device(tname, None) {
            return Err(lex::error(file, line, INPUT_REFUSED));
        }
        let ExprKind::Seq(sname) = &items[0].kind else {
            return Err(lex::error(file, items[0].line, format!("a line at feature scope pushing into '{}$' is an edge, `{}$ << x$`, and its first item is a stream; items are pushed on the declaration, `{} {}$ << ...`", tname, tname, zero_ty(telem), tname)));
        };
        if cond.is_some() {
            return Err(lex::error(file, line, "an edge has no `while`: it moves every item its stream receives"));
        }
        if sname == tname {
            return Err(lex::error(file, line, format!("'{}$' would feed itself", tname)));
        }
        let Some(sf) = self.fvar(sname).cloned() else {
            return Err(lex::error(file, items[0].line, format!("'{}$' is not a feature-scope stream: an edge reads one", sname)));
        };
        let Ty::Stream(selem) = &sf.ty else {
            return Err(lex::error(file, items[0].line, format!("'{}$' is a {}, not a stream", sname, sf.ty.ir())));
        };
        self.reach(&format!("{}$", tname), &tf.feature, file, line)?;
        self.reach(&format!("{}$", sname), &sf.feature, file, items[0].line)?;
        let name = format!("__edge{}", self.edges.len() + 1);
        let seq = |n: &str| Expr { kind: ExprKind::Seq(n.to_string()), line };
        // out of a stream with no storage (question 50, fm3 log 92) the
        // edge is a function of one item, its body the chain with the
        // item first, lowered as a plain function's push: a push into
        // the stream calls it, and there is no node
        if self.bare.contains(sname) {
            let mut pushed = vec![Expr { kind: ExprKind::Name("__item".into()), line }];
            pushed.extend(items[1..].iter().cloned());
            let fd = FnDecl {
                line,
                results: Vec::new(),
                name: vec![NamePart::Word(name.clone()), NamePart::Group],
                groups: vec![vec![super::syntax::Param { ty: zero_ty(selem), name: "__item".into(), seq: false, line }]],
                task: false,
                body: vec![Stmt::Push { target: seq(tname), items: pushed, cond: None, existing: false, line }],
                platform: Vec::new(),
            };
            self.declare(&fd, feature, file)?;
            let i = self.funcs.len() - 1;
            self.funcs[i].ir = name.clone();
            self.funcs[i].plain = name.clone();
            self.bare_edges.entry(sname.clone()).or_default().push((name, feature.to_string()));
            self.edges.push((fd, feature.to_string(), file.to_string()));
            return Ok(());
        }
        let phrase = |parts: Vec<Part>| Expr { kind: ExprKind::Phrase(parts), line };
        // `for __item in i$` with the pushes, then the reader moved past
        // what it read (log 75): one view and one advance move a batch
        let count = phrase(vec![Part::Word("count".into()), Part::Value(seq(sname))]);
        let mut pushed = vec![Expr { kind: ExprKind::Name("__item".into()), line }];
        pushed.extend(items[1..].iter().cloned());
        let advance = phrase(vec![Part::Word("advance".into()), Part::Value(seq(sname)), Part::Word("by".into()), Part::Args(vec![Arg { name: None, value: count }])]);
        let body = vec![Stmt::Push { target: seq(tname), items: pushed, cond: None, existing: false, line }];
        let fd = FnDecl {
            line,
            results: Vec::new(),
            name: vec![NamePart::Word(name.clone()), NamePart::Group],
            groups: vec![vec![super::syntax::Param { ty: zero_ty(selem), name: sname.clone(), seq: true, line }]],
            task: false,
            body: vec![Stmt::For { var: "__item".into(), seq: seq(sname), body, line }, Stmt::Expr { expr: advance, line }],
            platform: Vec::new(),
        };
        self.declare(&fd, feature, file)?;
        let i = self.funcs.len() - 1;
        self.funcs[i].ir = name.clone();
        self.funcs[i].plain = name.clone();
        self.funcs[i].task = true;
        self.edge_fns.insert(name, sname.clone());
        let info = self.funcs[i].clone();
        self.node_inputs.insert(sname.clone());
        let text = format!("{}$ << {}", tname, items.iter().map(phrase_text).collect::<Vec<_>>().join(" << "));
        self.nodes.push(Node { info, out: None, args: vec![items[0].clone()], hz: 0, feature: feature.to_string(), file: file.to_string(), text });
        self.edges.push((fd, feature.to_string(), file.to_string()));
        Ok(())
    }

    /// a feature-scope variable: a field of the context (log 16)
    fn declare_var(&mut self, v: &super::syntax::VarDecl, feature: &str, file: &str) -> Result<(), Error> {
        if v.name == "enabled" {
            return Err(lex::error(file, v.line, "every feature has 'enabled' already: a feature may not declare its own"));
        }
        if let Some(other) = self.fvars.iter().find(|f| f.name == v.name) {
            return Err(lex::error(file, v.line, format!("'{}' is already a variable of feature {}", v.name, other.feature)));
        }
        if v.scope.len() > 1 {
            return Err(lex::error(file, v.line, "a variable has one scope word: static, device or group"));
        }
        let ty = self.decl_ty(v, None, file)?;
        self.fvars.push(FVar {
            name: v.name.clone(),
            ty,
            scope: v.scope.first().cloned().unwrap_or_else(|| "user".into()),
            merge: v.merge.clone().unwrap_or_else(|| "last".into()),
            feature: feature.to_string(),
        });
        Ok(())
    }

    fn fvar(&self, name: &str) -> Option<&FVar> {
        self.fvars.iter().find(|f| f.name == name)
    }

    /// the context struct, its storage, its accessors, `__zero_reset`,
    /// which puts every variable's initial value in it, and the
    /// scheduler with a function per node (log 25)
    fn emit_context(&mut self, store: &Store) -> Result<(), Error> {
        let mut b = Body { out: String::new(), ntmp: 0, vars: HashMap::new(), defs: HashMap::new(), results: Vec::new(), file: String::new(), depth: 0, loops: Vec::new(), kind: BodyKind::Reset, func: None, below: None, product_bound: None };
        b.line("a: ptr = addr __arena");
        b.line("h: ptr = addr __heap");
        b.line("arena_init(a, h, 65536)");
        b.line("k: ptr = addr __clock");
        b.line("store 0: i64, k");
        // the node graph (log 78): what each node reads, what it may push
        // into, and an order with every producer before its consumers.
        // Settled before the context, since a node of an acyclic graph
        // asks less of its input (fm3 log 102) and one its pushers wake
        // keeps less in it (fm3 log 103)
        let schedule = if self.nodes.is_empty() { None } else { self.schedule(store, &self.nodes) };
        self.static_schedule = matches!(&schedule, Some((_, _, None)));
        // which nodes their pushers wake: under the static schedule, a
        // node that is not wired at a rate, whose task takes one stream
        // and nothing else, that stream being one only plain functions
        // push into and end (`Beat::woken`) and no node may push into,
        // and every node before it in the stream's order being woken
        // too, so the order of the pass stands. A stream with a rate
        // triggers after each item; a statement into it keeps a trailing
        // trigger too only where an item goes by a `<<` method, so no
        // method may take its type
        let mut woken: std::collections::HashSet<usize> = std::collections::HashSet::new();
        self.guard = true;
        if let Some((order, writes, None)) = &schedule {
            let (streams, guard) = Beat::new(self, store).woken(store);
            self.guard = guard;
            let mut inputs: Vec<String> = self.node_inputs.iter().cloned().collect();
            inputs.sort();
            for s in inputs {
                let rated = store.features.iter().any(|f| f.code.decls.iter().any(|d| matches!(d, Decl::Var(v) if v.name == s && v.rate.is_some())));
                let sty = self.fvar(&s).map(|f| f.ty.clone());
                let method = self.funcs.iter().any(|g| matches!(g.parts.as_slice(), [NamePart::Group, NamePart::Sym(op), NamePart::Group] if op == "<<") && sty.as_ref().is_some_and(|t| g.params.first().is_some_and(|p| fits(t, &p.1))));
                let may = streams.contains(&s) && !writes.iter().any(|w| w.contains(&s)) && !(rated && method);
                let (mut wakes, mut rest) = (Vec::new(), Vec::new());
                for k in self.reached(&s, &self.nodes, order, writes) {
                    let node = &self.nodes[k];
                    let one = node.info.params.len() == 1 && matches!(node.info.params[0].1, Ty::Stream(_)) && matches!(&node.args[0].kind, ExprKind::Seq(a) if a == &s);
                    if may && one && node.hz == 0 && rest.is_empty() {
                        wakes.push(k);
                        woken.insert(k);
                    } else {
                        rest.push(k);
                    }
                }
                self.wakes.insert(s.clone(), wakes);
                self.rests.insert(s, rest);
            }
        }
        // ... and of those, which keep a position alone (fm3 log 111)
        for &k in &woken {
            let info = &self.nodes[k].info;
            let param = info.params[0].0.clone();
            let mut defs = store.features.iter().flat_map(|f| f.code.decls.iter()).filter_map(|d| match d {
                Decl::Fn(fd) if mangle(&fd.name) == info.key && fd.name == info.parts && fd.params().count() == info.params.len() => Some(fd),
                _ => None,
            }).peekable();
            if defs.peek().is_some() && defs.all(|fd| fd.platform.is_empty() && self.ring_kept(&fd.body, &param)) {
                self.placed.insert(k);
            }
        }
        let kept = self.nodes.len() - woken.len();
        // a store with no nodes for the scheduler to run has none to
        // guard (fm3 log 92), and neither has one where no trigger can
        // be met while a node runs
        if kept > 0 && self.guard {
            b.line("r: ptr = addr __running");
            b.line("store 0: i64, r");
        }
        // the marks of the last case go with its text, cleared while its
        // count of bytes still says how far they reach (fm3 log 95); the
        // lines leave again where nothing waits (`lower`)
        for l in MARKS_RESET {
            b.line(l);
        }
        b.line("o: ptr = addr __out_n");
        b.line("store 0: i64, o");
        // the real clock starts at the reset (log 77)
        if self.clock == super::store::Clock::Real {
            b.line("c0: i64 = __counter()");
            b.line("q: ptr = addr __base");
            b.line("store c0, q");
        }
        // a node's state: its own reader of each input, and whether it
        // has finished, as fields after the variables
        for (k, node) in self.nodes.iter().enumerate() {
            let mut fields = Vec::new();
            for (a, (pname, pty)) in node.args.iter().zip(&node.info.params) {
                if let (ExprKind::Seq(_), Ty::Stream(_)) = (&a.kind, pty) {
                    let ty = if self.placed.contains(&k) { Ty::Num("i64".into()) } else { pty.clone() };
                    fields.push((format!("__node{}_{}", k + 1, pname), ty));
                    // how many items the ring had when the node last ran
                    if !woken.contains(&k) {
                        fields.push((format!("__node{}_{}_seen", k + 1, pname), Ty::Num("i64".into())));
                    }
                }
            }
            if !woken.contains(&k) {
                fields.push((format!("__node{}_fin", k + 1), Ty::Bool));
            }
            for (name, ty) in fields {
                self.fvars.push(FVar { name, ty, scope: "node".into(), merge: "last".into(), feature: node.feature.clone() });
            }
        }
        // every dynamic feature's implicit `enabled` (section 5, log 28),
        // first; a static feature has no switch (log 71)
        let mut at = 0;
        for f in self.features.clone().iter() {
            if self.statics.contains(f) {
                continue;
            }
            self.fvars.insert(at, FVar { name: format!("__enabled_{}", f), ty: Ty::Bool, scope: "user".into(), merge: "last".into(), feature: f.clone() });
            at += 1;
        }
        if !self.fvars.is_empty() {
            let mut fields = Vec::new();
            self.type_lines.push(String::new());
            self.type_lines.push("; the context: one field per feature-scope variable — name: scope, merge (feature)".into());
            for f in &self.fvars {
                // the device is declared and has no storage (question 45)
                if self.device_var(f) {
                    self.type_lines.push(format!(";   {}: the output device, which stores nothing ({})", f.name, f.feature));
                    continue;
                }
                // ... and so is a stream no word reads (question 50)
                if self.bare.contains(&f.name) {
                    if self.bare_edges.contains_key(&f.name) {
                        self.type_lines.push(format!(";   {}: no storage, no word reading it: a push into it calls its edges ({})", f.name, f.feature));
                    } else {
                        self.type_lines.push(format!(";   {}: no storage, nothing in the program reading it or wiring it ({})", f.name, f.feature));
                    }
                    continue;
                }
                self.type_lines.push(format!(";   {}: {}, {} ({})", f.name, f.scope, f.merge, f.feature));
                fields.push(format!("{}: {}", f.name, f.ty.ir()));
            }
            self.type_lines.push(format!("type __ctx = struct\n    {}", fields.join("\n    ")));
            self.data.push("data __ctx_mem: array(__ctx, 1)".into());
            // the initial values, in composition order: every feature on,
            // then the variables, then the nodes' state
            let mut inits: Vec<String> = self.features.iter().filter(|f| !self.statics.contains(*f)).map(|_| "1".to_string()).collect();
            let mut init_of: HashMap<String, String> = HashMap::new();
            for feat in &store.features {
                for d in &feat.code.decls {
                    let Decl::Var(v) = d else { continue };
                    b.file = feat.code.file.clone();
                    self.cur = feat.name.clone();
                    if self.fvar(&v.name).is_some_and(|f| self.device_var(f)) || self.bare.contains(&v.name) {
                        continue;
                    }
                    let ty = self.fvar(&v.name).unwrap().ty.clone();
                    let val = match &v.init {
                        _ if matches!(ty, Ty::Stream(_)) => {
                            let wired = matches!(&v.init, Some(Init::Value(e)) if matches!(self.task_call(e, None, &b.file), Ok(Some(_))));
                            // wired to a task at a rate: timed by the rate (log 73)
                            if let Some(Init::Value(e)) = &v.init {
                                if matches!(self.task_call(e, None, &b.file), Ok(Some((_, _, hz))) if hz > 0) {
                                    self.timed.insert(v.name.clone());
                                    self.rated.insert(v.name.clone());
                                }
                            }
                            match &v.init {
                                // a stream with its items resident: the ring
                                // the expression made (log 38)
                                Some(Init::Value(e)) if !wired => {
                                    self.resident_init(v, &ty, e, &mut b)?
                                }
                                Some(Init::Pushes { items, cond }) => {
                                    let s = self.empty_stream(v, &ty, &mut b, None)?;
                                    // the items before the first task call
                                    // are pushed here; the calls are nodes
                                    let n = items.iter().position(|e| matches!(self.task_call(e, None, &b.file), Ok(Some(_)))).unwrap_or(items.len());
                                    self.lower_pushes(&v.name, &s, &items[..n], cond.as_ref(), &mut b)?;
                                    s
                                }
                                Some(Init::Construct(_)) => return Err(lex::error(&b.file, v.line, format!("'{}$' is a stream: it is filled with `<<`, or made from a list or a range", v.name))),
                                // the platform's `out$` (log 57): a regular ring of
                                // OUT_BYTES, a byte's tick its index, so a push
                                // costs no clock stamp
                                None if feat.name == "platform" && v.name == "out" => {
                                    self.regular.insert(v.name.clone());
                                    self.make_stream_cap(&ty, CLOCK_HZ, "regular", OUT_BYTES, &mut b, None)
                                }
                                // the platform's `in$` (log 62): a sparse ring of
                                // IN_BYTES, a keyboard being sparse on the clock
                                None if feat.name == "platform" && v.name == "in" => {
                                    let maker = self.flavour(Some("in"), &b);
                                    self.make_stream_cap(&ty, CLOCK_HZ, maker, IN_BYTES, &mut b, None)
                                }
                                _ => self.empty_stream(v, &ty, &mut b, None)?,
                            }
                        }
                        None => self.zero_val(&ty, &mut b),
                        Some(Init::Value(e)) => {
                            let val = self.lower_expr(e, Some(&ty), &mut b, None)?;
                            if !(val.ty == ty || (val.literal && fits_literal(&val, &ty))) {
                                return Err(lex::error(&b.file, e.line, format!("'{}' is {} but the value is {}", v.name, ty.ir(), val.ty.ir())));
                            }
                            val
                        }
                        Some(Init::Construct(args)) => {
                            let Ty::Struct(name) = &ty else {
                                return Err(lex::error(&b.file, v.line, format!("'{}' is not a struct to construct", v.ty)));
                            };
                            self.construct(&name.clone(), args, &mut b, None, v.line)?
                        }
                        Some(Init::Pushes { .. }) => unreachable!(),
                    };
                    init_of.insert(v.name.clone(), val.text.clone());
                    inits.push(val.text);
                }
            }
            // a node's readers start where its inputs' rings start (a
            // reader is a value: a copy is its own position)
            for (k, node) in self.nodes.iter().enumerate() {
                for (a, (_, pty)) in node.args.iter().zip(&node.info.params) {
                    if let (ExprKind::Seq(n), Ty::Stream(_)) = (&a.kind, pty) {
                        if self.placed.contains(&k) {
                            let at = b.tmp();
                            b.line(&format!("{}: i64 = get {}, pos", at, init_of[n]));
                            inits.push(at);
                        } else {
                            inits.push(init_of[n].clone());
                        }
                        if !woken.contains(&k) {
                            inits.push("0".into());
                        }
                    }
                }
                if !woken.contains(&k) {
                    inits.push("0".into());
                }
            }
            let c = b.tmp();
            b.line(&format!("{}: __ctx = pack {}", c, inits.join(", ")));
            b.line("p: ptr = addr __ctx_mem");
            b.line(&format!("store {}, p", c));
        }
        b.line("ret");
        writeln!(self.out, "\n; before every case: the arena emptied, the variables at their initial values").unwrap();
        writeln!(self.out, "fn __zero_reset()").unwrap();
        self.out.push_str(&b.out);
        // the case's context is set between the reset and the start, so
        // a node of a feature that is off never runs (log 43)
        // ... a node its pushers wake has nothing to run on then (fm3 log 103)
        if kept > 0 {
            writeln!(self.out, "\n; after the case's context is set: the nodes run\nfn __zero_start()\n    __run()\n    ret").unwrap();
        }
        for f in &self.fvars {
            if self.device_var(f) || self.bare.contains(&f.name) {
                continue;
            }
            let t = f.ty.ir();
            writeln!(self.out, "\nfn __get_{}() -> {}\n    p: ptr = addr __ctx_mem\n    c: __ctx = load p\n    v: {} = get c, {}\n    ret v", f.name, t, t, f.name).unwrap();
            writeln!(self.out, "\nfn __set_{}(v: {})\n    p: ptr = addr __ctx_mem\n    c: __ctx = load p\n    c2: __ctx = set c, {}, v\n    store c2, p\n    ret", f.name, t, f.name).unwrap();
        }
        // a feature is on when its own flag and every ancestor's are
        // (section 14, log 51), and a switch writes one field, so a
        // parent off and on again leaves its children as they were. A
        // gate reads the flags in line (`gate`, fm3 log 110); these and
        // the accessors above are for a `platform` body of the store's
        // own, and the prune drops whichever nothing calls
        // ... a static feature is always on and reads nothing: a dynamic
        // feature under one reads its own flag and its nearest dynamic
        // ancestor's (log 71)
        if self.features.iter().any(|f| !self.statics.contains(f)) {
            writeln!(self.out, "\n; a feature's effective state: its own enabled and its ancestors', read by every gate").unwrap();
        }
        for f in &self.features {
            if self.statics.contains(f) {
                continue;
            }
            match self.dynamic_ancestor(f) {
                Some(p) => writeln!(self.out, "fn __on_{}() -> u1\n    own: u1 = __get___enabled_{}()\n    up: u1 = __on_{}()\n    on: u1 = and own, up\n    ret on", f, f, p).unwrap(),
                None => writeln!(self.out, "fn __on_{}() -> u1\n    own: u1 = __get___enabled_{}()\n    ret own", f, f).unwrap(),
            }
        }
        let mut counts: HashMap<String, usize> = HashMap::new();
        for n in &self.nodes {
            for a in &n.args {
                if let ExprKind::Seq(x) = &a.kind {
                    *counts.entry(x.clone()).or_insert(0) += 1;
                }
            }
        }
        self.node_reads = counts;
        let nodes = std::mem::take(&mut self.nodes);
        for (k, node) in nodes.iter().enumerate() {
            if woken.contains(&k) {
                let reads = match &node.args[0].kind {
                    ExprKind::Seq(n) => n.as_str(),
                    _ => unreachable!(),
                };
                writeln!(self.out, "\n; node {}: {} — woken by its pushers (fm3 log 103): a push into {}$ from a plain function, or its first end, calls the task there", k + 1, node.text, reads).unwrap();
                continue;
            }
            self.emit_node(k + 1, node)?;
        }
        if let Some((order, writes, cycle)) = &schedule {
            if let Some(through) = cycle {
                writeln!(self.out, "\n; the scheduler (log 25): passes over the nodes in declaration order until a pass runs nothing — the node graph has a cycle through {}$ (log 78)", through).unwrap();
            } else {
                let _ = writes;
                // one entry: the guard round the nodes' calls where a
                // trigger can be met while a node runs, and the calls
                // alone where none can (fm3 log 103)
                let entry = |name: &str, ks: &[usize], guard: bool| -> String {
                    let mut f = format!("fn {}()\n", name);
                    let pad = if guard { "        " } else { "    " };
                    if guard {
                        f.push_str("    p: ptr = addr __running\n    busy: i64 = load p\n    idle: u1 = cmp.eq busy, 0\n    if idle\n        store 1: i64, p\n");
                    }
                    for &k in ks {
                        writeln!(f, "{}r{}: u1 = __node{}()", pad, k + 1, k + 1).unwrap();
                    }
                    if guard {
                        f.push_str("        store 0: i64, p\n");
                    }
                    f.push_str("    ret");
                    f
                };
                let start: Vec<usize> = order.iter().copied().filter(|k| !woken.contains(k)).collect();
                if !start.is_empty() {
                    let which = if woken.is_empty() { "every node" } else { "every node its pushers do not wake" };
                    let guarded = if self.guard { "" } else { "; no trigger can be met while a node runs, so there is no guard, and a trigger that reaches one node is that node's call (fm3 log 103)" };
                    writeln!(self.out, "\n; the scheduler (log 25, 78): the node graph is acyclic, so one pass in producer-before-consumer order settles it — {} at the start, and after a push into a stream the nodes it reaches{}", which, guarded).unwrap();
                    writeln!(self.out, "{}", entry("__run", &start, self.guard)).unwrap();
                }
                let mut inputs: Vec<&String> = self.node_inputs.iter().collect();
                inputs.sort();
                for s in inputs {
                    let rest = &self.rests[s];
                    if rest.is_empty() || (!self.guard && rest.len() == 1) {
                        continue;
                    }
                    writeln!(self.out, "{}", entry(&format!("__run_{}", s), rest, self.guard)).unwrap();
                }
            }
        }
        if !nodes.is_empty() && !self.static_schedule {
            writeln!(self.out, "fn __run()\n    p: ptr = addr __running\n    busy: i64 = load p\n    idle: u1 = cmp.eq busy, 0\n    if idle\n        store 1: i64, p\n        loop()").unwrap();
            let mut any = String::new();
            for k in 1..=nodes.len() {
                writeln!(self.out, "            r{}: u1 = __node{}()", k, k).unwrap();
                if k == 1 {
                    any = "r1".into();
                } else {
                    writeln!(self.out, "            any{}: u1 = or {}, r{}", k, any, k).unwrap();
                    any = format!("any{}", k);
                }
            }
            writeln!(self.out, "            if {}\n                continue\n            else\n                break\n        store 0: i64, p\n    ret", any).unwrap();
        }
        self.nodes = nodes;
        Ok(())
    }

    /// the links of every chain (log 28): `key` is the outermost, and
    /// `key__before_F` the chain below feature F's body; each reads its
    /// feature's `enabled` and calls the body or the link below; the
    /// innermost, off, gives the results' zeros
    fn emit_links(&mut self) {
        let chains: Vec<FnInfo> = self.funcs.iter().filter(|f| f.chain.len() > 1).cloned().collect();
        for info in chains {
            let n = info.chain.len();
            let params: Vec<String> = info.params.iter().map(|(p, t)| format!("{}: {}", p, t.ir())).collect();
            let args: Vec<String> = info.params.iter().map(|(p, _)| p.clone()).collect();
            let rets: Vec<String> = info.results.iter().map(|(_, t)| t.ir()).collect();
            let sig_ret = match rets.len() {
                0 => String::new(),
                1 => format!(" -> {}", rets[0]),
                _ => format!(" -> ({})", rets.join(", ")),
            };
            if info.chain.iter().all(|f| self.statics.contains(f)) {
                // every feature static: the bodies call each other by name
                continue;
            }
            writeln!(self.out, "\n; {}: the chain {}, newest outermost; a link whose feature is off falls through", info.ir, info.chain.iter().rev().cloned().collect::<Vec<_>>().join(", ")).unwrap();
            for i in (0..n).rev() {
                if self.statics.contains(&info.chain[i]) {
                    // a static feature's body stands where its link would (log 71)
                    continue;
                }
                let name = link_name(&info, i, &self.statics);
                let body = format!("{}({})", body_name(&info, i, &self.statics), args.join(", "));
                let under = if i == 0 { None } else { Some(format!("{}({})", link_name(&info, i - 1, &self.statics), args.join(", "))) };
                let mut b = Body { out: String::new(), ntmp: 0, vars: HashMap::new(), defs: HashMap::new(), results: Vec::new(), file: String::new(), depth: 0, loops: Vec::new(), kind: BodyKind::Node, func: None, below: None, product_bound: None };
                self.gate(&info.chain[i].clone(), Some("on"), &mut b);
                if rets.is_empty() {
                    b.line("if on");
                    b.depth += 1;
                    b.line(&body);
                    b.depth -= 1;
                    if let Some(u) = under {
                        b.line("else");
                        b.depth += 1;
                        b.line(&u);
                        b.depth -= 1;
                    }
                    b.line("ret");
                } else {
                    let outs: Vec<String> = (0..rets.len()).map(|_| b.tmp()).collect();
                    let defs: Vec<String> = outs.iter().zip(&rets).map(|(o, t)| format!("{}: {}", o, t)).collect();
                    b.line(&format!("{} = if on", defs.join(", ")));
                    b.depth += 1;
                    let vs: Vec<String> = (0..rets.len()).map(|_| b.tmp()).collect();
                    let ds: Vec<String> = vs.iter().zip(&rets).map(|(v, t)| format!("{}: {}", v, t)).collect();
                    b.line(&format!("{} = {}", ds.join(", "), body));
                    b.line(&format!("yield {}", vs.join(", ")));
                    b.depth -= 1;
                    b.line("else");
                    b.depth += 1;
                    match under {
                        Some(u) => {
                            let vs: Vec<String> = (0..rets.len()).map(|_| b.tmp()).collect();
                            let ds: Vec<String> = vs.iter().zip(&rets).map(|(v, t)| format!("{}: {}", v, t)).collect();
                            b.line(&format!("{} = {}", ds.join(", "), u));
                            b.line(&format!("yield {}", vs.join(", ")));
                        }
                        None => {
                            let mut zs = Vec::new();
                            for (_, t) in &info.results {
                                zs.push(self.zero_val(t, &mut b).text);
                            }
                            b.line(&format!("yield {}", zs.join(", ")));
                        }
                    }
                    b.depth -= 1;
                    b.line(&format!("ret {}", outs.join(", ")));
                }
                writeln!(self.out, "fn {}({}){}", name, params.join(", "), sig_ret).unwrap();
                self.out.push_str(&b.out);
            }
            // ... and the same chain over the output device (log 90): a
            // `<<` method is lowered twice (log 87), so a redefinition
            // of it has two chains, and a feature that watches what is
            // written sees it whichever copy the call site chose
            let Some(dev) = self.device_fns.get(&info.ir).cloned() else { continue };
            let dparams: Vec<String> = info.params.iter().skip(1).map(|(p, t)| format!("{}: {}", p, t.ir())).collect();
            let dargs: Vec<String> = info.params.iter().skip(1).map(|(p, _)| p.clone()).collect();
            writeln!(self.out, "\n; {}: the same chain over the output device, where the method's stream is not a value (log 87, 90)", dev).unwrap();
            for i in (0..n).rev() {
                if self.statics.contains(&info.chain[i]) {
                    continue;
                }
                let name = named_link(&dev, &dev, &info.chain, i, &self.statics);
                let body = format!("{}({})", named_body(&dev, &dev, &info.chain, i, &self.statics), dargs.join(", "));
                let under = if i == 0 { None } else { Some(format!("{}({})", named_link(&dev, &dev, &info.chain, i - 1, &self.statics), dargs.join(", "))) };
                let mut b = Body { out: String::new(), ntmp: 0, vars: HashMap::new(), defs: HashMap::new(), results: Vec::new(), file: String::new(), depth: 0, loops: Vec::new(), kind: BodyKind::Node, func: None, below: None, product_bound: None };
                self.gate(&info.chain[i].clone(), Some("on"), &mut b);
                b.line("if on");
                b.depth += 1;
                b.line(&body);
                b.depth -= 1;
                if let Some(u) = under {
                    b.line("else");
                    b.depth += 1;
                    b.line(&u);
                    b.depth -= 1;
                }
                b.line("ret");
                writeln!(self.out, "fn {}({})", name, dparams.join(", ")).unwrap();
                self.out.push_str(&b.out);
            }
        }
    }

    /// `fn __nodeK() -> u1`: run the node when it is pending, and say
    /// whether it ran. Pending: an input has more items than it had
    /// when the node last ran (a node may leave what it cannot take yet,
    /// a partial token, unread), or has ended and the node has not run
    /// since; a node with no inputs, once. After a run the node is
    /// finished when all its inputs have ended.
    fn emit_node(&mut self, k: usize, node: &Node) -> Result<(), Error> {
        let mut b = Body { out: String::new(), ntmp: 0, vars: HashMap::new(), defs: HashMap::new(), results: Vec::new(), file: node.file.clone(), depth: 0, loops: Vec::new(), kind: BodyKind::Node, func: None, below: None, product_bound: None };
        self.field_get(&format!("__node{}_fin", k), "u1", Some("fin"), &mut b);
        b.line("notfin: u1 = xor fin, 1");
        let mut readers = Vec::new();
        // per reader: what its stream has received, and whether that is
        // more than the node has seen
        let mut behind: Vec<(String, String)> = Vec::new();
        // ... and whether it had ended
        let mut ends: Vec<String> = Vec::new();
        let mut pending: Option<String> = None;
        for (a, (pname, pty)) in node.args.iter().zip(&node.info.params) {
            let Ty::Stream(_) = pty else { continue };
            let r = self.field_get(&format!("__node{}_{}", k, pname), &pty.ir(), None, &mut b);
            let first = r.clone();
            let pushed = self.pushed_of(&first, &mut b);
            let seen = self.field_get(&format!("__node{}_{}_seen", k, pname), "i64", None, &mut b);
            let some = b.tmp();
            b.line(&format!("{}: u1 = cmp.gt {}, {}", some, pushed, seen));
            let e = b.tmp();
            b.line(&format!("{}: u1 = ended({})", e, first));
            let en = b.tmp();
            b.line(&format!("{}: u1 = and {}, notfin", en, e));
            let p = b.tmp();
            b.line(&format!("{}: u1 = or {}, {}", p, some, en));
            pending = Some(match pending {
                None => p,
                Some(q) => {
                    let pq = b.tmp();
                    b.line(&format!("{}: u1 = or {}, {}", pq, q, p));
                    pq
                }
            });
            let sname = match &a.kind {
                ExprKind::Seq(n) => Some(n.clone()),
                _ => None,
            };
            behind.push((pushed, some));
            ends.push(e);
            readers.push((pname.clone(), r, pty.clone(), sname));
        }
        let pending = pending.unwrap_or_else(|| "notfin".into());
        // a feature that is off runs no node; its readers keep their
        // place. A static feature is never off (log 71)
        let due = if self.statics.contains(&node.feature) {
            pending
        } else {
            self.gate(&node.feature, Some("on"), &mut b);
            b.line(&format!("due: u1 = and {}, on", pending));
            "due".to_string()
        };
        b.line(&format!("ran: u1 = if {}", due));
        b.depth += 1;
        let mut ops = Vec::new();
        if let Some(out) = &node.out {
            ops.push(self.read_fvar(out, &mut b, None, 0)?.text);
        }
        let mut ri = 0;
        for (a, (_, pty)) in node.args.iter().zip(&node.info.params) {
            if let Ty::Stream(_) = pty {
                ops.push(readers[ri].1.clone());
                ri += 1;
            } else {
                ops.push(self.plain_arg(&node.info, a, pty, &mut b)?);
            }
        }
        ops.push(format!("{}: i64", node.hz));
        let call = format!("{}({})", node.info.ir, ops.join(", "));
        let mut moved = Vec::new();
        for (_, _, ty, _) in readers.iter() {
            let r2 = b.tmp();
            moved.push(format!("{}: {}", r2, ty.ir()));
        }
        if moved.is_empty() {
            b.line(&call);
        } else {
            b.line(&format!("{} = {}", moved.join(", "), call));
        }
        let mut done: Option<String> = None;
        for (i, ((pname, _, _, sname), m)) in readers.iter().zip(&moved).enumerate() {
            let r2 = m.split(':').next().unwrap().to_string();
            self.field_put(&format!("__node{}_{}", k, pname), &r2, &mut b);
            // the queue's slots come back where this node is its one
            // reader (log 89, question 48)
            if sname.as_deref().is_some_and(|n| self.frees(n)) {
                b.line(&format!("free_queue({})", r2));
            }
            // what the node has now seen of its input. Where the graph
            // is acyclic no node pushes into or ends what it reads, that
            // being what acyclic is computed from (log 78), so the input
            // holds after the run what it held before it and is asked
            // once (fm3 log 102); and what arrives from outside while
            // the task runs is then not counted as seen. A node of a
            // cyclic graph may feed itself, and asks again
            let (pushed, e) = if self.static_schedule {
                (behind[i].0.clone(), ends[i].clone())
            } else {
                let pushed = self.pushed_of(&r2, &mut b);
                let e = b.tmp();
                b.line(&format!("{}: u1 = ended({})", e, r2));
                (pushed, e)
            };
            self.field_put(&format!("__node{}_{}_seen", k, pname), &pushed, &mut b);
            done = Some(match done {
                None => e,
                Some(d) => {
                    let de = b.tmp();
                    b.line(&format!("{}: u1 = and {}, {}", de, d, e));
                    de
                }
            });
        }
        self.field_put(&format!("__node{}_fin", k), &done.unwrap_or_else(|| "1".into()), &mut b);
        b.line("yield 1");
        b.depth -= 1;
        b.line("else");
        b.depth += 1;
        // a consumer that is off holds nothing (question 51, fm3 log
        // 92): what was pushed toward it while it was off is gone for
        // it, its reader moved past everything, so it starts from now
        // when it comes back on and its producer never fills its queue
        if !self.statics.contains(&node.feature) {
            b.line("off: u1 = xor on, 1");
            for ((pname, r, ty, sname), (pushed, some)) in readers.iter().zip(&behind) {
                let drop = b.tmp();
                b.line(&format!("{}: u1 = and {}, off", drop, some));
                b.line(&format!("if {}", drop));
                b.depth += 1;
                let r2 = b.tmp();
                b.line(&format!("{}: {} = set {}, pos, {}", r2, ty.ir(), r, pushed));
                self.field_put(&format!("__node{}_{}", k, pname), &r2, &mut b);
                if sname.as_deref().is_some_and(|n| self.frees(n)) {
                    b.line(&format!("free_queue({})", r2));
                }
                self.field_put(&format!("__node{}_{}_seen", k, pname), pushed, &mut b);
                b.depth -= 1;
            }
        }
        b.line("yield 0");
        b.depth -= 1;
        b.line("ret ran");
        writeln!(self.out, "\n; node {}: {} — run when an input has more than the node has seen, or has ended and the node has not run since", k, node.text).unwrap();
        writeln!(self.out, "fn __node{}() -> u1", k).unwrap();
        self.out.push_str(&b.out);
        Ok(())
    }

    /// how many items a reader's ring has received: the ring's count
    /// (log 68: it was the reader's position plus what is unread, and
    /// `position` computes a tick the test throws away)
    fn pushed_of(&mut self, reader: &str, b: &mut Body) -> String {
        let pushed = b.tmp();
        b.line(&format!("{}: i64 = received({})", pushed, reader));
        pushed
    }

    /// a task's plain argument, lowered against its parameter
    fn plain_arg(&mut self, info: &FnInfo, a: &Expr, pty: &Ty, b: &mut Body) -> Result<String, Error> {
        let file = b.file.clone();
        let mut v = self.lower_expr(a, Some(pty), b, None)?;
        if v.literal && fits_literal(&v, pty) {
            v.ty = pty.clone();
        }
        if !fits(&v.ty, pty) {
            return Err(lex::error(&file, a.line, format!("'{}' wants a {} here, given a {}", info.key, pty.ir(), v.ty.ir())));
        }
        // a literal is defined with its type first: in a node, a plain IR
        // function, a bare literal to an abstract `int` has no type to take
        Ok(b.materialize(&v).text)
    }

    /// A field of the context read in place (fm3 log 110): the context
    /// loaded at its address and the field taken, which the IR
    /// dissolves to a load of the one field (log 67). `_this` is the
    /// context's address, formed once a function by `settle_context`;
    /// a zero name cannot begin with `_`, so no variable is called it
    fn field_get(&mut self, field: &str, ty: &str, dst: Option<&str>, b: &mut Body) -> String {
        let c = b.tmp();
        let out = match dst {
            Some(d) if b.vars.contains_key(d) => {
                let t = b.vars[d].ty.clone();
                b.define(d, t)
            }
            Some(d) => d.to_string(),
            None => b.tmp(),
        };
        b.line(&format!("{}: __ctx = load {}", c, THIS));
        b.line(&format!("{}: {} = get {}, {}", out, ty, c, field));
        out
    }

    /// A field of the context written in place: a load, a `set` and a
    /// store, dissolved to a store of the one field. Every store to the
    /// context a function makes is written here, so `written` is every
    /// field that changes while a case runs
    fn field_put(&mut self, field: &str, v: &str, b: &mut Body) {
        let (c1, c2) = (b.tmp(), b.tmp());
        b.line(&format!("{}: __ctx = load {}", c1, THIS));
        b.line(&format!("{}: __ctx = set {}, {}, {}", c2, c1, field, v));
        b.line(&format!("store {}, {}", c2, THIS));
        self.written.insert(field.to_string());
    }

    /// A feature's effective state read in line (log 51, 71, fm3 log
    /// 110): its own switch and each dynamic ancestor's, one load of the
    /// context, a `get` each and an `and` each after the first
    fn gate(&mut self, feature: &str, dst: Option<&str>, b: &mut Body) -> String {
        let mut chain = vec![feature.to_string()];
        while let Some(p) = self.dynamic_ancestor(chain.last().unwrap()) {
            chain.push(p);
        }
        let c = b.tmp();
        b.line(&format!("{}: __ctx = load {}", c, THIS));
        let mut acc = String::new();
        for (i, f) in chain.iter().enumerate() {
            let last = i + 1 == chain.len();
            let named = |b: &mut Body| match dst {
                Some(d) if b.vars.contains_key(d) => {
                    let t = b.vars[d].ty.clone();
                    b.define(d, t)
                }
                Some(d) => d.to_string(),
                None => b.tmp(),
            };
            let own = if last && i == 0 { named(b) } else { b.tmp() };
            b.line(&format!("{}: u1 = get {}, __enabled_{}", own, c, f));
            acc = if i == 0 {
                own
            } else {
                let both = if last { named(b) } else { b.tmp() };
                b.line(&format!("{}: u1 = and {}, {}", both, acc, own));
                both
            };
        }
        acc
    }

    /// a feature's switch read (log 51): its effective state, own flag
    /// and ancestors', read in line by `gate`
    fn read_on(&mut self, feature: &str, b: &mut Body, dst: Option<&str>) -> Val {
        if self.statics.contains(feature) {
            let out = name_for(dst, &Ty::Bool, b);
            b.line(&format!("{}: u1 = const 1", out));
            return Val { text: out, ty: Ty::Bool, literal: false };
        }
        let out = self.gate(feature, dst.filter(|d| b.vars.get(*d).map(|v| &v.ty) == Some(&Ty::Bool)), b);
        Val { text: out, ty: Ty::Bool, literal: false }
    }

    /// the nearest ancestor of a feature that is dynamic, whose state
    /// the feature's own gate conjoins with (log 71)
    fn dynamic_ancestor(&self, feature: &str) -> Option<String> {
        let mut p = self.parents.get(feature).cloned().flatten();
        while let Some(f) = p {
            if !self.statics.contains(&f) {
                return Some(f);
            }
            p = self.parents.get(&f).cloned().flatten();
        }
        None
    }

    /// a feature variable read: its field of the context, in place (fm3 log 110)
    fn read_fvar(&mut self, name: &str, b: &mut Body, dst: Option<&str>, line: usize) -> Result<Val, Error> {
        let f = self.fvar(name).unwrap().clone();
        self.reach(name, &f.feature, &b.file, line)?;
        // the device has no storage and so no value (question 45): the
        // push sites know it by name, and every other use is refused
        if self.device_var(&f) {
            return Ok(Val { text: "__device".into(), ty: f.ty, literal: false });
        }
        // ... and neither has a stream no word reads (question 50): only
        // a push names it, and a push into it is its edges' call
        if self.bare.contains(name) {
            return Ok(Val { text: "__bare".into(), ty: f.ty, literal: false });
        }
        let dst = dst.filter(|d| b.vars.get(*d).map(|v| &v.ty) == Some(&f.ty));
        let out = self.field_get(name, &f.ty.ir(), dst, b);
        Ok(Val { text: out, ty: f.ty, literal: false })
    }

    /// a feature variable written: its field stored in place (fm3 log 110)
    fn write_fvar(&mut self, name: &str, v: Val, b: &mut Body, line: usize) -> Result<(), Error> {
        let f = self.fvar(name).unwrap().clone();
        self.reach(name, &f.feature, &b.file, line)?;
        let ty = f.ty;
        let v = self.coerce(v, &ty, &format!("'{}'", name), b, None, line)?;
        self.field_put(name, &v.text, b);
        Ok(())
    }

    fn lower_fn(&mut self, f: &FnDecl, feature: &str, file: &str) -> Result<(), Error> {
        let key = mangle(&f.name);
        self.regular_locals.clear();
        // methods (log 36): the one declared for these parameter types
        let mut tys = Vec::new();
        for p in f.params() {
            tys.push(self.ty(&p.ty, p.seq, file, p.line)?);
        }
        let mut candidates: Vec<&FnInfo> = self.funcs.iter().filter(|g| g.key == key && g.parts == f.name && g.params.len() == f.params().count()).collect();
        if candidates.len() > 1 {
            candidates.retain(|g| g.params.iter().map(|(_, t)| t.clone()).collect::<Vec<Ty>>() == tys);
        }
        let info = candidates[0].clone();
        // a task's IR results are its stream parameters, moved on (log 25)
        let results: Vec<(String, Ty)> = if info.task { info.params.iter().filter(|(_, t)| matches!(t, Ty::Stream(_))).cloned().collect() } else { info.results.clone() };
        let kind = if info.task { BodyKind::Task { out: info.results.first().map(|(n, _)| n.clone()), hz: "__hz".into() } } else { BodyKind::Fn };
        // in a chain the body is `key__feature`, and `existing` is the link below
        let (name, below) = if info.chain.len() > 1 {
            let i = info.chain.iter().position(|c| c == feature).unwrap();
            (body_name(&info, i, &self.statics), if i == 0 { None } else { Some(link_name(&info, i - 1, &self.statics)) })
        } else {
            (info.plain.clone(), None)
        };
        let mut b = Body { out: String::new(), ntmp: 0, vars: HashMap::new(), defs: HashMap::new(), results: results.clone(), file: file.to_string(), depth: 0, loops: Vec::new(), kind, func: Some(info.clone()), below, product_bound: self.product.get(&key).copied() };
        let mut sig = format!("fn {}(", name);
        let mut sig_params: Vec<(String, Ty)> = Vec::new();
        if let (true, Some(r)) = (info.task, info.results.first()) {
            sig_params.push(r.clone());
        }
        sig_params.extend(info.params.iter().cloned());
        if info.task {
            sig_params.push(("__hz".into(), Ty::Num("i64".into())));
        }
        for (i, (n, t)) in sig_params.iter().enumerate() {
            if i > 0 {
                sig.push_str(", ");
            }
            write!(sig, "{}: {}", n, t.ir()).unwrap();
            if b.vars.contains_key(n) {
                return Err(lex::error(file, f.line, format!("parameter '{}' is named twice{}", n, if info.task { " (a task's result is its first parameter)" } else { "" })));
            }
            b.define(n, t.clone());
        }
        sig.push(')');
        match results.len() {
            0 => {}
            1 => write!(sig, " -> {}", results[0].1.ir()).unwrap(),
            _ => {
                let ts: Vec<String> = results.iter().map(|(_, t)| t.ir()).collect();
                write!(sig, " -> ({})", ts.join(", ")).unwrap();
            }
        }
        if !info.task {
            for (n, t) in &results {
                if b.vars.contains_key(n) {
                    return Err(lex::error(file, f.line, format!("result '{}' is also a parameter, or named twice", n)));
                }
                b.declare(n, t.clone());
            }
        }
        if let Some(n) = b.product_bound {
            writeln!(self.out, "; product setting: bound {}: {}", key.replace('_', " "), n).unwrap();
        }
        writeln!(self.out, "{}", sig).unwrap();
        if let Some(kinds) = &info.platform {
            return self.lower_platform(f, &info, kinds, &sig_params, &results, &mut b);
        }
        let terminated = self.lower_block(&f.body, &mut b)?;
        // the results' current values; one never assigned is its type's
        // zero. A body that ends in a loop with no way out never returns
        if !terminated {
            let mut rets = Vec::new();
            for (n, t) in &b.results.clone() {
                let v = b.vars[n].clone();
                if !v.set {
                    rets.push(self.zero_val(t, &mut b).text);
                } else {
                    rets.push(v.ir);
                }
            }
            b.line(format!("ret {}", rets.join(", ")).trim_end());
        }
        self.out.push_str(&b.out);
        self.lower_device_fn(f, &info, feature, file)
    }

    /// The device copy of a `<<` method (log 87, questions 44 and 45).
    /// `out$` is not a stream: a push into it is the platform's write,
    /// and inside `on (char o$) << (int x)` the front end cannot see
    /// which stream `o$` is. So the method is lowered a second time with
    /// its stream parameter dropped — `o$` is the device there, every
    /// push into it is the write, and a `<<` it calls in turn is that
    /// method's device copy — and the call site chooses the copy by
    /// whether its stream argument is the device. The prune (log 70)
    /// keeps only the copies a store reaches
    fn lower_device_fn(&mut self, f: &FnDecl, info: &FnInfo, feature: &str, file: &str) -> Result<(), Error> {
        let Some(dev) = self.device_fns.get(&info.ir).cloned() else { return Ok(()) };
        self.regular_locals.clear();
        let (sname, sty) = info.params[0].clone();
        // a redefinition applies to both copies (log 90): the device
        // copies chain under the device name, so `existing` inside one
        // reaches the definition below it there
        let (name, below) = if info.chain.len() > 1 {
            let i = info.chain.iter().position(|c| c == feature).unwrap();
            (named_body(&dev, &dev, &info.chain, i, &self.statics), if i == 0 { None } else { Some(named_link(&dev, &dev, &info.chain, i - 1, &self.statics)) })
        } else {
            (dev.clone(), None)
        };
        let mut b = Body {
            out: String::new(), ntmp: 0, vars: HashMap::new(), defs: HashMap::new(), results: Vec::new(),
            file: file.to_string(), depth: 0, loops: Vec::new(), kind: BodyKind::Fn, func: Some(info.clone()),
            below, product_bound: self.product.get(&info.key).copied(),
        };
        b.vars.insert(sname.clone(), Var { ir: "__device".into(), ty: sty, set: true, loop_depth: 0 });
        let mut sig = format!("fn {}(", name);
        for (i, (n, t)) in info.params.iter().skip(1).enumerate() {
            if i > 0 {
                sig.push_str(", ");
            }
            write!(sig, "{}: {}", n, t.ir()).unwrap();
            b.define(n, t.clone());
        }
        sig.push(')');
        writeln!(self.out, "; ... and the same over the output device, where a push is the platform's write (log 87)").unwrap();
        writeln!(self.out, "{}", sig).unwrap();
        self.device_param = Some(sname);
        let terminated = self.lower_block(&f.body, &mut b);
        self.device_param = None;
        if !terminated? {
            b.line("ret");
        }
        self.out.push_str(&b.out);
        Ok(())
    }

    /// A platform function's bodies (section 15, log 31). The `ir` body is
    /// the IR function's body; without one, the body prints that the
    /// function is out of reach and fails a check, which a target with
    /// a rule never runs. A body for a target is a rule in a `platform
    /// <target> { ... }` block after the function: the header from the
    /// signature, the lines as written. The signature's `{` is open
    fn lower_platform(&mut self, f: &FnDecl, info: &FnInfo, kinds: &[String], sig_params: &[(String, Ty)], results: &[(String, Ty)], b: &mut Body) -> Result<(), Error> {
        let file = b.file.clone();
        match f.platform.iter().find(|(ks, _)| ks.iter().any(|k| k == "ir")) {
            Some((_, lines)) => {
                for l in lines {
                    b.line(l);
                }
            }
            None => {
                let note = Expr { kind: ExprKind::Str(format!("no platform body for '{}' here", spoken(info))), line: f.line };
                let sv = self.lower_expr(&note, None, b, None)?;
                b.line(&format!("print({})", sv.text));
                let z = b.tmp();
                b.line(&format!("{}: u1 = const 0", z));
                b.line(&format!("check {}", z));
                let mut rets = Vec::new();
                for (_, t) in results {
                    rets.push(self.zero_val(t, b).text);
                }
                b.line(format!("ret {}", rets.join(", ")).trim_end());
            }
        }
        self.out.push_str(&b.out);
        let params: Vec<String> = sig_params.iter().map(|(n, t)| format!("{}: {}", n, t.ir())).collect();
        let ret = results.first().map(|(_, t)| t.ir()).unwrap_or_else(|| "()".into());
        for k in kinds {
            if k == "ir" {
                continue;
            }
            let (_, lines) = f.platform.iter().find(|(ks, _)| ks.contains(k)).unwrap();
            writeln!(self.out, "platform {}", k).unwrap();
            writeln!(self.out, "    {}({}) -> {}", info.ir, params.join(", "), ret).unwrap();
            for l in lines {
                writeln!(self.out, "        {}", l).unwrap();
            }
        }
        let _ = file;
        Ok(())
    }

    /// Lower a block's statements; true when the block ends by leaving:
    /// `break` or `continue`, an assignment that gives the function its
    /// last result (section 6: assigning the result ends the function),
    /// or an `if` or `loop` that always does. A statement after that
    /// would never run, and is refused here with its zero line rather
    /// than by the IR with an IR line (log 14).
    fn lower_block(&mut self, stmts: &[Stmt], b: &mut Body) -> Result<bool, Error> {
        let mut terminated = false;
        for (i, s) in stmts.iter().enumerate() {
            self.after_push = match i.checked_sub(1).map(|k| &stmts[k]) {
                Some(Stmt::Push { target: Expr { kind: ExprKind::Seq(n), .. }, existing: false, .. }) => Some(n.clone()),
                _ => None,
            };
            terminated = self.lower_stmt(s, b)?;
            if terminated && i + 1 < stmts.len() {
                let why = match s {
                    Stmt::Assign { line, .. } => format!("the function ended when its result was assigned on line {}", line),
                    _ => "the statement before it leaves the block".to_string(),
                };
                return Err(lex::error(&b.file, stmt_line(&stmts[i + 1]), format!("this never runs: {}", why)));
            }
        }
        Ok(terminated)
    }

    /// After an assignment in a function's body: when every result now
    /// has a value, the function ends here with `ret` (section 6). A
    /// task's body pushes its result and never ends this way
    fn finish_if_done(&mut self, b: &mut Body) -> bool {
        if b.kind != BodyKind::Fn || b.results.is_empty() {
            return false;
        }
        if !b.results.iter().all(|(n, _)| b.vars.get(n).map_or(false, |v| v.set)) {
            return false;
        }
        let names: Vec<String> = b.results.iter().map(|(n, _)| n.clone()).collect();
        let vals = b.current(&names);
        b.line(&format!("ret {}", vals.join(", ")));
        true
    }

    /// would assigning these targets give the function its last result?
    /// Such an assignment ends the function, so it may stand inside a
    /// loop, where an ordinary assignment to an outer variable may not
    fn completes(&self, targets: &[super::syntax::Target], b: &Body) -> bool {
        b.kind == BodyKind::Fn && !b.results.is_empty() && b.results.iter().all(|(n, _)| b.vars.get(n).map_or(false, |v| v.set) || targets.iter().any(|t| &t.name == n))
    }

    /// `if (c)` as a statement (log 11): the variables an arm assigns
    /// become the results of the IR's value-yielding `if`, each arm
    /// yielding its version, so the code after reads the join's names.
    /// True when both arms leave, so the `if` does
    fn lower_if(&mut self, cond: &Expr, then: &[Stmt], els: Option<&[Stmt]>, b: &mut Body) -> Result<bool, Error> {
        let file = b.file.clone();
        let cv = self.lower_expr(cond, Some(&Ty::Bool), b, None)?;
        if cv.ty != Ty::Bool {
            return Err(lex::error(&file, cond.line, "'if' takes a bool"));
        }
        let cv = b.materialize(&cv);
        let before = b.vars.clone();
        let start = b.out.len();
        b.depth += 1;
        let t_term = self.lower_block(then, b)?;
        let then_vars = std::mem::replace(&mut b.vars, before.clone());
        let mut then_lines = b.out.split_off(start);
        let (e_term, else_vars) = match els {
            Some(e) => {
                let t = self.lower_block(e, b)?;
                (t, std::mem::replace(&mut b.vars, before.clone()))
            }
            None => (false, before.clone()),
        };
        let mut else_lines = b.out.split_off(start);
        b.depth -= 1;
        // the join's results: the variables from before that an arm
        // which falls through has given a new value
        let differs = |arm: &HashMap<String, Var>, n: &str| arm[n].ir != before[n].ir || arm[n].set != before[n].set;
        let mut changed: Vec<String> = before.keys().filter(|n| (!t_term && differs(&then_vars, n)) || (!e_term && differs(&else_vars, n))).cloned().collect();
        changed.sort();
        // what each arm yields: its version, or the type's zero for a
        // variable that has none yet on that path
        let mut yields = |lines: &mut String, vars: &HashMap<String, Var>, b: &mut Body| -> Vec<String> {
            let saved = std::mem::replace(&mut b.out, std::mem::take(lines));
            b.depth += 1;
            let mut out = Vec::new();
            for n in &changed {
                let v = &vars[n];
                if v.set {
                    out.push(v.ir.clone());
                } else {
                    let ty = v.ty.clone();
                    out.push(self.zero_val(&ty, b).text);
                }
            }
            b.depth -= 1;
            *lines = std::mem::replace(&mut b.out, saved);
            out
        };
        let then_yields = if t_term { Vec::new() } else { yields(&mut then_lines, &then_vars, b) };
        let else_yields = if e_term { Vec::new() } else { yields(&mut else_lines, &else_vars, b) };
        let head = if changed.is_empty() {
            format!("if {}", cv.text)
        } else {
            let defs: Vec<String> = changed.iter().map(|n| {
                let ty = before[n].ty.clone();
                format!("{}: {}", b.define(n, ty.clone()), ty.ir())
            }).collect();
            format!("{} = if {}", defs.join(", "), cv.text)
        };
        b.line(&head);
        b.out.push_str(&then_lines);
        if !t_term && !changed.is_empty() {
            b.depth += 1;
            b.line(&format!("yield {}", then_yields.join(", ")));
            b.depth -= 1;
        }
        if els.is_some() || !changed.is_empty() {
            b.line("else");
            b.out.push_str(&else_lines);
            if !e_term && !changed.is_empty() {
                b.depth += 1;
                b.line(&format!("yield {}", else_yields.join(", ")));
                b.depth -= 1;
            }
        }
        Ok(t_term && e_term && els.is_some())
    }

    /// `loop (vars) while (c) yields x` (log 12, 40, 48): the carried
    /// variables are the header's and the loop's own — gone after it;
    /// `while` is tested at the top of every pass, a body that falls off
    /// its end continues with the current versions, and every `break`
    /// yields the given variables, which leave into the names `into`
    /// says, and the outer streams the body moved
    #[allow(clippy::too_many_arguments)]
    fn lower_loop(&mut self, vars: &[super::syntax::VarDecl], cond: Option<&Expr>, body: &[Stmt], yields: &[String], into: Option<&LoopInto>, line: usize, b: &mut Body) -> Result<bool, Error> {
        let file = b.file.clone();
        let beat = self.loop_beat.take();
        let mut firsts: Vec<(String, Ty, Val)> = Vec::new();
        // the initial values, in the block before the loop
        let mut header = Vec::new();
        let mut carried = Vec::new();
        let mut tys = Vec::new();
        for v in vars {
            if !v.scope.is_empty() || v.merge.is_some() {
                return Err(lex::error(&file, v.line, "a scope word or 'merge' belongs on a feature-scope variable"));
            }
            if b.vars.contains_key(&v.name) || carried.contains(&v.name) {
                return Err(lex::error(&file, v.line, format!("'{}' is already declared", v.name)));
            }
            let ty = self.ty(&v.ty, v.seq, &file, v.line)?;
            let init = match &v.init {
                None => self.zero_val(&ty, b),
                Some(Init::Value(e)) => {
                    let val = self.lower_expr(e, Some(&ty), b, None)?;
                    self.coerce(val, &ty, &format!("'{}'", v.name), b, None, e.line)?
                }
                Some(Init::Construct(args)) => {
                    let Ty::Struct(name) = &ty else {
                        return Err(lex::error(&file, v.line, format!("'{}' is not a struct to construct", v.ty)));
                    };
                    self.construct(&name.clone(), args, b, None, v.line)?
                }
                Some(Init::Pushes { .. }) => return Err(lex::error(&file, v.line, "a stream is declared before the loop, which then carries it")),
            };
            if v.rate.is_some() {
                return Err(lex::error(&file, v.line, "a stream is declared before the loop, which then carries it"));
            }
            firsts.push((v.name.clone(), ty.clone(), init.clone()));
            header.push((v.name.clone(), ty.clone(), init.text));
            carried.push(v.name.clone());
            tys.push(ty);
        }
        // what the loop yields: carried variables, each once (log 40)
        for (i, g) in yields.iter().enumerate() {
            let Some(k) = carried.iter().position(|c| c == g) else {
                return Err(lex::error(&file, line, format!("'{}' is not a variable the loop carries: `yields` names one of its header's, {}", g, if carried.is_empty() { "and this loop has none".to_string() } else { carried.join(", ") })));
            };
            let _ = k;
            if yields[..i].contains(g) {
                return Err(lex::error(&file, line, format!("'{}' is yielded twice", g)));
            }
        }
        let targets = match into {
            Some(LoopInto::Declare(ps)) => ps.len(),
            Some(LoopInto::Assign(ts)) => ts.len(),
            None => 0,
        };
        if into.is_some() && yields.is_empty() {
            return Err(lex::error(&file, line, "`= loop` names what the loop yields: `... while (c) yields acc`"));
        }
        if into.is_none() && !yields.is_empty() {
            return Err(lex::error(&file, line, format!("the loop yields '{}' to nothing: `int total = loop (...) ... yields {}`", yields[0], yields[0])));
        }
        if into.is_some() && targets != yields.len() {
            return Err(lex::error(&file, line, format!("the loop yields {} value(s) to {} name(s)", yields.len(), targets)));
        }
        // a stream the body moves (`advance`, `frame`) is carried too: its
        // position threads through the loop as an ordinary value (log 23),
        // and leaves it moved
        let mut moved = Vec::new();
        moved_streams(body, &mut moved, &|parts| self.task_streams(parts));
        let mut streams = Vec::new();
        for n in moved {
            if carried.contains(&n) {
                continue;
            }
            if let Some(v) = b.vars.get(&n) {
                if matches!(v.ty, Ty::Stream(_)) && v.set {
                    header.push((n.clone(), v.ty.clone(), v.ir.clone()));
                    carried.push(n.clone());
                    tys.push(v.ty.clone());
                    streams.push(n.clone());
                }
            }
        }
        let mut results: Vec<String> = yields.to_vec();
        results.extend(streams.iter().cloned());
        // the body's first statement pushes into a stream with a rate,
        // and every pass leaves the clock on that stream's beat
        // (question 56, fm3 log 99): the first pass's alignment is made
        // here, once, where there is a first pass — the `while` asked
        // of the initial values, which `Beat` has checked may be asked
        // twice. A loop of no passes pushes nothing and takes no time
        if let Some(hz) = beat {
            match cond {
                None => self.align(hz, b),
                Some(c) => {
                    let outer = b.vars.clone();
                    let depth = b.loops.len();
                    for (n, ty, init) in &firsts {
                        let v = b.materialize(init);
                        b.vars.insert(n.clone(), Var { ir: v.text, ty: ty.clone(), set: true, loop_depth: depth });
                    }
                    let cv = self.lower_expr(c, Some(&Ty::Bool), b, None)?;
                    let cv = b.materialize(&cv);
                    b.vars = outer;
                    b.line(&format!("if {}", cv.text));
                    b.depth += 1;
                    self.align(hz, b);
                    b.depth -= 1;
                }
            }
        }
        // the carried variables are declared inside the loop
        let outer = b.vars.clone();
        b.loops.push(LoopCtx { carried: carried.clone(), results: results.clone(), explicit: vars.len(), item: None, loaded: None, breaks: 0 });
        let mut hdr = Vec::new();
        let inner = b.loops.len();
        for (n, ty, init) in &header {
            let ir = b.define(n, ty.clone());
            // a stream carried from outside is the loop's own inside it
            b.vars.get_mut(n).unwrap().loop_depth = inner;
            hdr.push(format!("{}: {} = {}", ir, ty.ir(), init));
        }
        let start = b.out.len();
        b.depth += 1;
        if let Some(c) = cond {
            let cv = self.lower_expr(c, Some(&Ty::Bool), b, None)?;
            if cv.ty != Ty::Bool {
                return Err(lex::error(&file, c.line, "'while' takes a bool"));
            }
            let cv = b.materialize(&cv);
            b.line(&format!("if {}", cv.text));
            b.line("else");
            b.depth += 1;
            let vals = b.current(&results);
            b.line(format!("break {}", vals.join(", ")).trim_end());
            b.depth -= 1;
            b.loops.last_mut().unwrap().breaks += 1;
        }
        let terminated = self.lower_block(body, b)?;
        if !terminated {
            let vals = b.current(&carried);
            b.line(format!("continue {}", vals.join(", ")).trim_end());
        }
        b.depth -= 1;
        let body_lines = b.out.split_off(start);
        let ctx = b.loops.pop().unwrap();
        // after the loop its variables are gone; the outer scope stands
        b.vars = outer;
        // a loop with no way out has no results and nothing after it
        if ctx.breaks == 0 {
            if !yields.is_empty() {
                return Err(lex::error(&file, line, format!("the loop never leaves, so it yields nothing: a `while`, or a `break`, is how '{}' comes out", yields[0])));
            }
            b.open_loop("", &hdr.join(", "), false);
            b.out.push_str(&body_lines);
            return Ok(true);
        }
        // the results' names: a given value under its target's own name
        // when the types agree, a temporary widened or set after otherwise
        let depth = b.loops.len();
        let mut defs = Vec::new();
        let mut after: Vec<(String, Val, bool)> = Vec::new();
        for (i, g) in yields.iter().enumerate() {
            let gty = tys[carried.iter().position(|c| c == g).unwrap()].clone();
            let (name, tline, declared) = match into.unwrap() {
                LoopInto::Declare(ps) => {
                    let p = &ps[i];
                    if b.vars.contains_key(&p.name) {
                        return Err(lex::error(&file, p.line, format!("'{}' is already declared", p.name)));
                    }
                    let ty = self.ty(&p.ty, p.seq, &file, p.line)?;
                    b.declare(&p.name, ty);
                    (p.name.clone(), p.line, true)
                }
                LoopInto::Assign(ts) => {
                    let t = &ts[i];
                    if t.feature.is_some() {
                        return Err(lex::error(&file, t.line, "a loop yields values to variables"));
                    }
                    if b.vars.contains_key(&t.name) {
                        if !(self.completes(ts, b) && b.results.iter().any(|(n, _)| n == &t.name)) {
                            b.assignable(&t.name, t.line)?;
                        }
                    } else if self.fvar(&t.name).is_none() {
                        return Err(lex::error(&file, t.line, format!("'{}' is not declared: a variable is its type then its name", t.name)));
                    }
                    (t.name.clone(), t.line, false)
                }
            };
            let _ = declared;
            let direct = b.vars.get(&name).is_some_and(|v| v.ty == gty);
            if direct {
                let ir = b.define(&name, gty.clone());
                b.vars.get_mut(&name).unwrap().loop_depth = depth;
                defs.push(format!("{}: {}", ir, gty.ir()));
            } else {
                let t = b.tmp();
                defs.push(format!("{}: {}", t, gty.ir()));
                let local = b.vars.contains_key(&name);
                after.push((name, Val { text: t, ty: gty, literal: false }, local));
                let _ = tline;
            }
        }
        for n in &streams {
            let ty = outer_ty(&b.vars, n, self);
            let ir = if b.vars.contains_key(n) {
                let ir = b.define(n, ty.clone());
                b.vars.get_mut(n).unwrap().loop_depth = depth;
                ir
            } else {
                let t = b.tmp();
                after.push((n.clone(), Val { text: t.clone(), ty: ty.clone(), literal: false }, false));
                t
            };
            defs.push(format!("{}: {}", ir, ty.ir()));
        }
        // a loop that yields nothing and moves no stream stands alone
        let prefix = if defs.is_empty() { String::new() } else { format!("{} = ", defs.join(", ")) };
        b.open_loop(&prefix, &hdr.join(", "), false);
        b.out.push_str(&body_lines);
        for (name, v, local) in after {
            if local {
                self.assign(&name, v, b, line)?;
            } else {
                self.write_fvar(&name, v, b, line)?;
            }
        }
        Ok(matches!(into, Some(LoopInto::Assign(_))) && self.finish_if_done(b))
    }

    /// `for (x in [a through b])`, `[a to b]` (log 13): a loop whose
    /// carried variable is the item, stepped by one each pass. Literal
    /// bounds fix the direction, and the IR is the plain counted loop
    /// `probe cost` reads; otherwise the step is chosen at run time and
    /// the test is on the signed distance left
    fn lower_for(&mut self, var: &str, seq: &Expr, body: &[Stmt], b: &mut Body) -> Result<(), Error> {
        let file = b.file.clone();
        self.no_moved_stream(body, b)?;
        let ExprKind::Range { from, to, inclusive } = &seq.kind else {
            return self.lower_for_seq(var, seq, body, b);
        };
        if b.vars.contains_key(var) {
            return Err(lex::error(&file, seq.line, format!("'{}' is already declared", var)));
        }
        for e in [from, to] {
            if matches!(e.kind, ExprKind::Float(_)) {
                return Err(lex::error(&file, e.line, "a range's bounds are integers"));
            }
        }
        let mut fv = self.lower_expr(from, None, b, None)?;
        let mut tv = self.lower_expr(to, if fv.literal { None } else { Some(&fv.ty) }, b, None)?;
        if fv.literal && !tv.literal {
            fv.ty = tv.ty.clone();
        }
        if tv.literal && !fv.literal {
            tv.ty = fv.ty.clone();
        }
        let ty = fv.ty.clone();
        if !matches!(ty, Ty::Num(_)) || ty != tv.ty {
            return Err(lex::error(&file, seq.line, format!("a range runs over one number type, given {} and {}", fv.ty.ir(), tv.ty.ir())));
        }
        let counted = fv.literal && tv.literal;
        // the direction, and the test that leaves
        let (op, step, test) = if fv.literal && tv.literal {
            let a: i64 = fv.text.parse().map_err(|_| lex::error(&file, from.line, "a range's bounds are integers"))?;
            let z: i64 = tv.text.parse().map_err(|_| lex::error(&file, to.line, "a range's bounds are integers"))?;
            let up = a <= z;
            let cc = match (up, *inclusive) {
                (true, true) => "cmp.le",
                (true, false) => "cmp.lt",
                (false, true) => "cmp.ge",
                (false, false) => "cmp.gt",
            };
            (if up { "add" } else { "sub" }, "1".to_string(), Some(format!("{} {{}}, {}", cc, tv.text)))
        } else {
            let Ty::Num(n) = &ty else { unreachable!() };
            if !(n == "int" || (n.starts_with('i') && n[1..].parse::<u32>().is_ok())) {
                return Err(lex::error(&file, seq.line, format!("a range with a bound that is not a literal counts over a signed integer, not {}", n)));
            }
            let fv2 = b.materialize(&fv);
            let tv2 = b.materialize(&tv);
            fv = fv2;
            tv = tv2;
            let d = b.tmp();
            b.line(&format!("{}: {} = sub {}, {}", d, ty.ir(), tv.text, fv.text));
            let down = b.tmp();
            b.line(&format!("{}: u1 = cmp.lt {}, 0", down, d));
            let step = b.tmp();
            b.line(&format!("{}: {} = if {}", step, ty.ir(), down));
            b.depth += 1;
            b.line("yield -1");
            b.depth -= 1;
            b.line("else");
            b.depth += 1;
            b.line("yield 1");
            b.depth -= 1;
            ("add", step, None)
        };
        b.loops.push(LoopCtx { carried: Vec::new(), results: Vec::new(), explicit: 0, item: Some((var.to_string(), op, step.clone())), loaded: None, breaks: 1 });
        let x = b.define(var, ty.clone());
        let before = b.vars.clone();
        let start = b.out.len();
        b.depth += 1;
        let done = match &test {
            Some(t) => {
                let done = b.tmp();
                b.line(&format!("{}: u1 = {}", done, t.replace("{}", &x)));
                done
            }
            None => {
                let left = b.tmp();
                b.line(&format!("{}: {} = sub {}, {}", left, ty.ir(), tv.text, x));
                let signed = b.tmp();
                b.line(&format!("{}: {} = mul {}, {}", signed, ty.ir(), left, step));
                let done = b.tmp();
                b.line(&format!("{}: u1 = {} {}, 0", done, if *inclusive { "cmp.lt" } else { "cmp.le" }, signed));
                done
            }
        };
        if test.is_some() {
            b.line(&format!("if {}", done));
            b.line("else");
            b.depth += 1;
            b.line("break");
            b.depth -= 1;
        } else {
            b.line(&format!("if {}", done));
            b.depth += 1;
            b.line("break");
            b.depth -= 1;
        }
        let terminated = self.lower_block(body, b)?;
        if !terminated {
            self.step_for(b);
        }
        b.depth -= 1;
        let body_lines = b.out.split_off(start);
        b.loops.pop();
        b.vars = before;
        b.vars.remove(var);
        b.open_loop("", &format!("{}: {} = {}", x, ty.ir(), fv.text), counted);
        b.out.push_str(&body_lines);
        Ok(())
    }

    /// `for (x in items$)`: a loop over the unread items by index, the
    /// reader not moved, the item peeked at the top of each pass and
    /// gone after the loop (log 38)
    fn lower_for_seq(&mut self, var: &str, seq: &Expr, body: &[Stmt], b: &mut Body) -> Result<(), Error> {
        let file = b.file.clone();
        let sv = self.lower_expr(seq, None, b, None)?;
        let Some(e) = sv.ty.elem().cloned() else {
            return Err(lex::error(&file, seq.line, format!("a `for` runs over a stream or a range, not a {}", sv.ty.ir())));
        };
        if b.vars.contains_key(var) {
            return Err(lex::error(&file, seq.line, format!("'{}' is already declared", var)));
        }
        // the unread items as one view, a load per item (log 75)
        let view = Some(self.unread_view(&sv, b));
        let n = b.tmp();
        match &view {
            Some(v) => b.line(&format!("{}: i64 = len {}", n, v)),
            None => {
                let first = sv.text.clone();
                b.line(&format!("{}: i64 = count {}", n, first));
            }
        }
        // an edge over a rated source takes each item at its tick (log
        // 77): the item's index is the reader's position plus the loop's
        let paced = match (&b.func, &seq.kind) {
            // a source declared with a rate needs no wait here (log 93):
            // its item was pushed at its time, and the edge runs in that
            // push's trigger. A rated task's output is still waited for
            (Some(f), ExprKind::Seq(src)) if self.edge_fns.get(&f.ir) == Some(src) && self.rated.contains(src) && !self.rates.contains_key(src) => Some(src.clone()),
            _ => None,
        };
        let pos = b.tmp();
        if paced.is_some() {
            b.line(&format!("{}: i64 = get {}, pos", pos, sv.text));
        }
        let k = b.tmp();
        b.loops.push(LoopCtx { carried: Vec::new(), results: Vec::new(), explicit: 0, item: Some((k.clone(), "add", "1".into())), loaded: Some(var.to_string()), breaks: 1 });
        let depth = b.loops.len();
        b.vars.insert(k.clone(), Var { ir: k.clone(), ty: Ty::Num("i64".into()), set: true, loop_depth: depth });
        let before = b.vars.clone();
        let start = b.out.len();
        b.depth += 1;
        let done = b.tmp();
        b.line(&format!("{}: u1 = cmp.ge {}, {}", done, k, n));
        b.line(&format!("if {}", done));
        b.depth += 1;
        b.line("break");
        b.depth -= 1;
        b.declare(var, e.clone());
        match &view {
            Some(v) => {
                let out = name_for(Some(var), &e, b);
                b.line(&format!("{}: {} = load {}, {}", out, e.ir(), v, k));
            }
            None => {
                self.peek_at(&sv, &k, b, Some(var));
            }
        }
        if paced.is_some() {
            let abs = b.tmp();
            b.line(&format!("{}: i64 = add {}, {}", abs, pos, k));
            // a rated task's output: its ticks are the store's clock's
            let t = b.tmp();
            b.line(&format!("{}: i64 = tick_of({}, {})", t, sv.text, abs));
            b.line(&format!("__wait({})", t));
        }
        let terminated = self.lower_block(body, b)?;
        if !terminated {
            self.step_for(b);
        }
        b.depth -= 1;
        let body_lines = b.out.split_off(start);
        b.loops.pop();
        b.vars = before;
        b.vars.remove(var);
        b.vars.remove(&k);
        // an edge's loop is bounded by the most items one event pushes
        // into its source (log 79), when every such push was countable
        let arrival = match (&b.func, &seq.kind) {
            (Some(f), ExprKind::Seq(src)) if self.edge_fns.get(&f.ir) == Some(src) => self.arrivals.get(src).copied().flatten(),
            _ => None,
        };
        match arrival {
            Some(n) => {
                if let ExprKind::Seq(src) = &seq.kind {
                    b.line(&format!("; the most items one event pushes into {}$: {} (log 79)", src, n));
                }
                b.line(&format!("loop({}: i64 = 0) bound {}", k, n));
            }
            None => b.open_loop("", &format!("{}: i64 = 0", k), false),
        }
        b.out.push_str(&body_lines);
        Ok(())
    }

    /// a `for`'s pass ends: the item stepped, and `continue` with it
    fn step_for(&mut self, b: &mut Body) {
        let (var, op, step) = b.loops.last().unwrap().item.clone().unwrap();
        let v = b.vars[&var].clone();
        let next = b.tmp();
        b.line(&format!("{}: {} = {} {}, {}", next, v.ty.ir(), op, v.ir, step));
        b.line(&format!("continue {}", next));
    }

    /// a type's zero: a literal for a number, a bool or an enumeration,
    /// the empty string, a struct of its defaults
    fn zero_val(&mut self, t: &Ty, b: &mut Body) -> Val {
        match t {
            Ty::Bool | Ty::Num(_) | Ty::Enum(_) | Ty::Char => Val { text: "0".into(), ty: t.clone(), literal: true },
            Ty::Struct(name) => self.construct(name, &[], b, None, 0).unwrap_or(Val { text: "0".into(), ty: t.clone(), literal: true }),
            // an empty stream
            Ty::Stream(_) => {
                let maker = self.flavour(None, b);
                self.make_stream(t, CLOCK_HZ, maker, b, None)
            }
            Ty::None => Val { text: String::new(), ty: Ty::None, literal: false },
        }
    }

    /// a struct from its arguments, by position or by name, the rest
    /// from the fields' defaults: `Vec(1, 2, 3)`, `Vec v(z = 3)`, `Vec v`
    fn construct(&mut self, name: &str, args: &[Arg], b: &mut Body, dst: Option<&str>, line: usize) -> Result<Val, Error> {
        let file = b.file.clone();
        let Some(TypeInfo::Struct(fields)) = self.types.get(name).cloned() else {
            return Err(lex::error(&file, line, format!("'{}' is not a struct", name)));
        };
        let named = args.iter().any(|a| a.name.is_some());
        if named && args.iter().any(|a| a.name.is_none()) {
            return Err(lex::error(&file, line, "arguments are all by position or all by name"));
        }
        if !named && args.len() > fields.len() {
            return Err(lex::error(&file, line, format!("{} has {} field(s), given {}", name, fields.len(), args.len())));
        }
        for a in args.iter().filter_map(|a| a.name.as_ref()) {
            if !fields.iter().any(|(n, _, _)| n == a) {
                return Err(lex::error(&file, line, format!("{} has no field '{}'", name, a)));
            }
        }
        let mut ops = Vec::new();
        for (i, (fname, fty, default)) in fields.iter().enumerate() {
            let given = if named { args.iter().find(|a| a.name.as_deref() == Some(fname)) } else { args.get(i) };
            let v = match given {
                Some(a) => {
                    let v = self.lower_expr(&a.value, Some(fty), b, None)?;
                    self.coerce(v, fty, &format!("field '{}' of {}", fname, name), b, None, a.value.line)?
                }
                None => match default {
                    Some(d) => Val { text: d.clone(), ty: fty.clone(), literal: true },
                    None => self.zero_val(fty, b),
                },
            };
            ops.push(v.text);
        }
        let ty = Ty::Struct(name.to_string());
        let out = name_for(dst, &ty, b);
        b.line(&format!("{}: {} = pack {}", out, name, ops.join(", ")));
        Ok(Val { text: out, ty, literal: false })
    }

    /// a value converted to a wider number type (log 37): the IR's
    /// `conv`, under `dst`'s next version when it is that type; a
    /// conversion that can lose bits carries a note above it (log 46),
    /// the warning the ruling allows
    fn widen_to(&mut self, v: &Val, to: &Ty, b: &mut Body, dst: Option<&str>) -> Val {
        if &v.ty == to {
            return v.clone();
        }
        if v.literal {
            return Val { text: v.text.clone(), ty: to.clone(), literal: true };
        }
        if let Some(note) = loses(&v.ty, to) {
            b.line(&format!("; {}", note));
        }
        let name = name_for(dst, to, b);
        b.line(&format!("{}: {} = conv {}", name, to.ir(), v.text));
        Val { text: name, ty: to.clone(), literal: false }
    }

    /// The value a place of type `ty` takes from `v` (question 1, log
    /// 37; log 46): the value itself, a literal retyped, or a value
    /// widened by a `conv` where the place's type is what the pair
    /// computes in. Refused naming the explicit form where the place is
    /// the narrower type, where a third type holds both better, or
    /// where nothing converts an abstract or library type
    fn coerce(&mut self, v: Val, ty: &Ty, what: &str, b: &mut Body, dst: Option<&str>, line: usize) -> Result<Val, Error> {
        if v.ty == *ty {
            return Ok(v);
        }
        if v.literal {
            if fits_literal(&v, ty) {
                return Ok(Val { text: v.text, ty: ty.clone(), literal: true });
            }
            return Err(lex::error(&b.file, line, format!("{} is {} but the value is a decimal", what, zero_ty(ty))));
        }
        if widens(&v.ty, ty) {
            return Ok(self.widen_to(&v, ty, b, dst));
        }
        let how = match wider(&v.ty, ty) {
            Some(w) if w == v.ty => "narrows it, which is not implied".to_string(),
            Some(w) => format!("is not implied: {} holds both better", zero_ty(&w)),
            None => "converts it; nothing widens it".to_string(),
        };
        Err(lex::error(&b.file, line, format!("{} is {} but the value is {}: {}(...) {}", what, zero_ty(ty), zero_ty(&v.ty), zero_ty(ty), how)))
    }

    /// assign a value to a declared variable: a literal becomes a
    /// `const` under the variable's name; a value that already has a
    /// name is not copied — the variable names it too; a narrower
    /// number is widened (log 37)
    fn assign(&mut self, name: &str, v: Val, b: &mut Body, line: usize) -> Result<(), Error> {
        let var = b.vars[name].clone();
        if v.literal {
            if !fits_literal(&v, &var.ty) {
                return Err(lex::error(&b.file, line, format!("'{}' is {} but the value is {}", name, var.ty.ir(), v.ty.ir())));
            }
            let ir = b.define(name, var.ty.clone());
            b.line(&format!("{}: {} = const {}", ir, var.ty.ir(), v.text));
            return Ok(());
        }
        let v = self.coerce(v, &var.ty, &format!("'{}'", name), b, Some(name), line)?;
        if var.ir != v.text {
            let v2 = b.vars.get_mut(name).unwrap();
            v2.ir = v.text;
            v2.set = true;
        }
        Ok(())
    }

    /// Lower one statement; true when it ends the block (see `lower_block`)
    fn lower_stmt(&mut self, s: &Stmt, b: &mut Body) -> Result<bool, Error> {
        let file = b.file.clone();
        match s {
            Stmt::Assign { targets, value, line } => {
                // `countdown.enabled = false` is not in the language (log
                // 43): the chooser switches a feature, a case line says
                // `with countdown off`
                if let [t] = targets.as_slice() {
                    if let Some(feat) = &t.feature {
                        if t.name != "enabled" || !self.features.contains(feat) {
                            return Err(lex::error(&file, t.line, format!("'{}.{}': a feature's implicit variable is `{}.enabled`", feat, t.name, feat)));
                        }
                        return Err(lex::error(&file, t.line, format!("'{}.enabled' is not assigned in feature code: a feature is switched by the chooser, and a case says `with {} off` on its line (section 14)", feat, feat)));
                    }
                }
                if targets.iter().any(|t| t.feature.is_some()) {
                    return Err(lex::error(&file, *line, "a feature's `enabled` is assigned on its own"));
                }
                // a local is a new SSA version; a feature variable is a
                // store of its field, allowed anywhere; the assignment
                // that gives the last result ends the function, so it
                // may stand anywhere too
                let completes = self.completes(targets, b);
                for t in targets {
                    if b.vars.contains_key(&t.name) {
                        if !(completes && b.results.iter().any(|(n, _)| n == &t.name)) {
                            b.assignable(&t.name, t.line)?;
                        }
                    } else if self.fvar(&t.name).is_none() {
                        return Err(lex::error(&file, t.line, format!("'{}' is not declared: a variable is its type then its name", t.name)));
                    }
                }
                if targets.len() == 1 {
                    let t = &targets[0];
                    if !b.vars.contains_key(&t.name) {
                        let ty = self.fvar(&t.name).unwrap().ty.clone();
                        let v = self.lower_expr(value, Some(&ty), b, None)?;
                        self.write_fvar(&t.name, v, b, *line)?;
                        return Ok(false);
                    }
                    let ty = b.vars[&t.name].ty.clone();
                    let v = self.lower_expr(value, Some(&ty), b, Some(&t.name))?;
                    self.assign(&t.name, v, b, *line)?;
                    return Ok(self.finish_if_done(b));
                }
                let names: Vec<String> = targets.iter().map(|t| t.name.clone()).collect();
                let tys: Vec<Ty> = names.iter().map(|n| match b.vars.get(n) { Some(v) => v.ty.clone(), None => self.fvar(n).unwrap().ty.clone() }).collect();
                self.lower_multi(value, &names, &tys, b, *line)?;
                Ok(self.finish_if_done(b))
            }
            Stmt::Multi { vars, value, line } => {
                let mut names = Vec::new();
                let mut tys = Vec::new();
                for p in vars {
                    if b.vars.contains_key(&p.name) {
                        return Err(lex::error(&file, p.line, format!("'{}' is already declared", p.name)));
                    }
                    let ty = self.ty(&p.ty, p.seq, &file, p.line)?;
                    b.declare(&p.name, ty.clone());
                    names.push(p.name.clone());
                    tys.push(ty);
                }
                self.lower_multi(value, &names, &tys, b, *line)?;
                Ok(false)
            }
            Stmt::Var(v) => {
                if !v.scope.is_empty() || v.merge.is_some() {
                    return Err(lex::error(&file, v.line, "a scope word or 'merge' belongs on a feature-scope variable"));
                }
                if b.vars.contains_key(&v.name) {
                    return Err(lex::error(&file, v.line, format!("'{}' is already declared", v.name)));
                }
                let ty = self.decl_ty(v, Some(&b.vars), &file)?;
                b.declare(&v.name, ty.clone());
                if let Ty::Stream(_) = &ty {
                    // a stream (log 38): the ring an expression made with
                    // its items resident; or an empty ring, then the chain
                    // of pushes, or the task that fills it, run now (log 25)
                    let task = match &v.init {
                        Some(Init::Value(e)) => self.task_call(e, Some(&b.vars), &file)?,
                        _ => None,
                    };
                    match (&v.init, task) {
                        (Some(Init::Value(e)), None) => {
                            let s = self.resident_init(v, &ty, e, b)?;
                            self.assign(&v.name, s, b, v.line)?;
                        }
                        (Some(Init::Value(e)), Some((info, args, hz))) => {
                            if hz > 0 {
                                self.timed.insert(v.name.clone());
                            }
                            let s = self.empty_stream(v, &ty, b, Some(&v.name))?;
                            self.assign(&v.name, s.clone(), b, v.line)?;
                            self.run_task(&info, &args, hz, &s, b, e.line)?;
                        }
                        (Some(Init::Pushes { items, cond }), _) => {
                            let s = self.empty_stream(v, &ty, b, Some(&v.name))?;
                            self.assign(&v.name, s.clone(), b, v.line)?;
                            self.lower_pushes(&v.name, &s, items, cond.as_ref(), b)?;
                        }
                        (Some(Init::Construct(_)), _) => return Err(lex::error(&file, v.line, format!("'{}$' is a stream: it is filled with `<<`, or made from a list or a range", v.name))),
                        (None, _) => {
                            let s = self.empty_stream(v, &ty, b, Some(&v.name))?;
                            self.assign(&v.name, s, b, v.line)?;
                        }
                    }
                    return Ok(false);
                }
                match &v.init {
                    None => {
                        let z = self.zero_val(&ty, b);
                        self.assign(&v.name, z, b, v.line)?
                    }
                    Some(Init::Value(e)) => {
                        let val = self.lower_expr(e, Some(&ty), b, Some(&v.name))?;
                        self.assign(&v.name, val, b, v.line)?
                    }
                    Some(Init::Construct(args)) => {
                        let Ty::Struct(name) = &ty else {
                            return Err(lex::error(&file, v.line, format!("'{}' is not a struct to construct", v.ty)));
                        };
                        let val = self.construct(&name.clone(), args, b, Some(&v.name), v.line)?;
                        self.assign(&v.name, val, b, v.line)?
                    }
                    Some(Init::Pushes { .. }) => unreachable!(),
                }
                Ok(false)
            }
            Stmt::Expr { expr, line } => {
                match &expr.kind {
                    ExprKind::Phrase(_) | ExprKind::Existing(_) => {}
                    _ => return Err(lex::error(&file, *line, "a statement is a call, an assignment or a declaration")),
                }
                self.lower_expr(expr, None, b, None)?;
                Ok(false)
            }
            Stmt::If { cond, then, els, .. } => self.lower_if(cond, then, els.as_deref(), b),
            Stmt::Loop { vars, cond, body, yields, into, line } => {
                self.loop_beat = self.loop_beats.get(&(s as *const Stmt as usize)).copied();
                self.lower_loop(vars, cond.as_ref(), body, yields, into.as_ref(), *line, b)
            }
            Stmt::For { var, seq, body, .. } => {
                self.lower_for(var, seq, body, b)?;
                Ok(false)
            }
            Stmt::Break { line } => {
                if b.loops.is_empty() {
                    return Err(lex::error(&file, *line, "'break' outside a loop"));
                }
                let ctx = b.loops.last_mut().unwrap();
                ctx.breaks += 1;
                let results = ctx.results.clone();
                let vals = b.current(&results);
                b.line(format!("break {}", vals.join(", ")).trim_end());
                Ok(true)
            }
            Stmt::Continue { values, line } => {
                let Some(ctx) = b.loops.last() else {
                    return Err(lex::error(&file, *line, "'continue' outside a loop"));
                };
                if ctx.item.is_some() {
                    if !values.is_empty() {
                        return Err(lex::error(&file, *line, "a `for` steps its item by itself: 'continue' takes no values here"));
                    }
                    self.step_for(b);
                    return Ok(true);
                }
                let carried = ctx.carried.clone();
                let explicit = ctx.explicit;
                if values.is_empty() {
                    let vals = b.current(&carried);
                    b.line(format!("continue {}", vals.join(", ")).trim_end());
                    return Ok(true);
                }
                if values.len() != explicit {
                    return Err(lex::error(&file, *line, format!("the loop declares {} variable(s), 'continue' gives {}", explicit, values.len())));
                }
                let mut vals = Vec::new();
                for (e, n) in values.iter().zip(&carried) {
                    let ty = b.vars[n].ty.clone();
                    let v = self.lower_expr(e, Some(&ty), b, None)?;
                    let v = self.coerce(v, &ty, &format!("'{}'", n), b, None, e.line)?;
                    vals.push(v.text);
                }
                // a stream the loop carries for the body goes on as it stands
                vals.extend(b.current(&carried[explicit..]));
                b.line(&format!("continue {}", vals.join(", ")));
                Ok(true)
            }
            Stmt::Check { cond, line } => {
                // `check (c)` (section 14, log 29): the IR's trap when c
                // does not hold, the site printed first so the runner
                // can name it
                let cv = self.lower_expr(cond, Some(&Ty::Bool), b, None)?;
                if cv.ty != Ty::Bool {
                    return Err(lex::error(&file, *line, "'check' takes a bool"));
                }
                let cv = b.materialize(&cv);
                b.line(&format!("if {}", cv.text));
                b.line("else");
                b.depth += 1;
                let base = file.rsplit('/').next().unwrap_or(&file).to_string();
                let site = Expr { kind: ExprKind::Str(format!("check at {}:{}", base, line)), line: *line };
                let sv = self.lower_expr(&site, None, b, None)?;
                b.line(&format!("print({})", sv.text));
                let z = b.tmp();
                b.line(&format!("{}: u1 = const 0", z));
                b.line(&format!("check {}", z));
                b.depth -= 1;
                Ok(false)
            }
            Stmt::Push { target, items, cond, existing, line } => {
                let ExprKind::Seq(n) = &target.kind else {
                    return Err(lex::error(&file, *line, "`<<` pushes into a stream, named `x$`"));
                };
                if *existing {
                    return self.existing_push(n, &items[0], b, *line).map(|_| false);
                }
                if self.input_device(n, Some(b)) {
                    return Err(lex::error(&file, *line, INPUT_REFUSED));
                }
                if self.stream_var(n, b).is_none() {
                    return Err(match self.seq_or_fvar_ty(n, b) {
                        Some(t) => lex::error(&file, *line, format!("'{}$' is a {}, not a stream", n, t.ir())),
                        None => lex::error(&file, *line, format!("'{}$' is not declared", n)),
                    });
                }
                // into a stream with a rate the statement's first item
                // lands on the stream's beat (question 52, fm3 log 98),
                // unless the statement before left now there, or the
                // clock is known to be on that beat here (question 56)
                let on_beat = self.after_push.take().as_deref() == Some(n.as_str()) || self.on_beat.contains(&(s as *const Stmt as usize));
                let rate = if self.is_bare(n, b) { self.rates.get(n).copied() } else { self.paced(n, b) };
                if let (Some(hz), false) = (rate, on_beat) {
                    self.align(hz, b);
                }
                let s = self.lower_expr(&Expr { kind: ExprKind::Name(n.clone()), line: *line }, None, b, None)?;
                // into a stream with no storage (question 50): each
                // edge's gate is read once, before the items, a feature
                // being switched at the next event and not in the
                // middle of a statement
                let bare = self.is_bare(n, b);
                if bare {
                    let gates = self.read_gates(n, b);
                    self.bare_gates = Some((n.clone(), gates));
                }
                self.loose_push = false;
                self.sure_push = false;
                self.push_site = Some((n.clone(), b.depth));
                let mark = b.out.len();
                let done = self.lower_pushes(n, &s, items, cond.as_ref(), b);
                self.push_site = None;
                if bare {
                    self.bare_gates = None;
                }
                done?;
                // a paced push has triggered after each item (log 93)
                if self.paced(n, b).is_none() || self.loose_push {
                    // the nodes the statement wakes (fm3 log 103) are due
                    // if it pushed anything, as `received` over `seen`
                    // said: a single item pushed at the statement's own
                    // depth is known to have, and otherwise `received`
                    // is read before the statement and after it. The
                    // read before is written here, the lowering only now
                    // knowing that it is needed
                    if self.wakes_here(n, b) {
                        if self.paced(n, b).is_some() {
                            return Err(lex::error(&file, *line, format!("'{}$' has a rate and nodes its pushers wake, and this statement pushes through a method or a task: the front end should not have chosen to wake them (fm3 log 103)", n)));
                        }
                        if self.sure_push {
                            self.wake(n, &s.text, true, b);
                        } else {
                            let before = b.tmp();
                            b.out.insert_str(mark, &format!("{}{}: i64 = received({})\n", "    ".repeat(b.depth + 1), before, s.text));
                            let after = self.pushed_of(&s.text, b);
                            let more = b.tmp();
                            b.line(&format!("{}: u1 = cmp.gt {}, {}", more, after, before));
                            b.line(&format!("if {}", more));
                            b.depth += 1;
                            self.wake(n, &s.text, true, b);
                            b.depth -= 1;
                        }
                    }
                    self.trigger(n, b);
                }
                Ok(false)
            }
        }
    }

    /// `existing name(args)` (log 28): the link below this body in its
    /// chain, called with the arguments; the phrase names the enclosing
    /// function
    fn existing_call(&mut self, parts: &[Part], b: &mut Body, line: usize) -> Result<(String, Vec<Ty>), Error> {
        let file = b.file.clone();
        let Some(info) = b.func.clone() else {
            return Err(lex::error(&file, line, "'existing' belongs in a function's body"));
        };
        let Some(below) = b.below.clone() else {
            return Err(lex::error(&file, line, format!("no earlier definition of '{}' for 'existing' to call{}", spoken(&info), if info.chain.len() > 1 { ": this is the first" } else { "" })));
        };
        let is_var = |w: &str| b.vars.contains_key(w) || self.fvar(w).is_some();
        let (named, args) = find_methods(std::slice::from_ref(&info), parts, &is_var, &file, line).map_err(|_| lex::error(&file, line, format!("'existing' names the function it is in: `existing {}(...)`", spoken(&info))))?;
        let named = named[0].clone();
        let (ops, rtys) = self.lower_args(&named, &args, b)?;
        Ok((format!("{}({})", below, ops.join(", ")), rtys))
    }

    /// `existing o$ << x` (question 46): the link below this body in
    /// its chain, called with the stream and the item — the shape
    /// `existing name(...)` cannot spell, a `<<` method's name being an
    /// operator. The device copy of the method has its own chain, and
    /// there the call takes the item alone, `o$` not being a value
    fn existing_push(&mut self, name: &str, item: &Expr, b: &mut Body, line: usize) -> Result<(), Error> {
        let file = b.file.clone();
        let Some(info) = b.func.clone() else {
            return Err(lex::error(&file, line, "'existing' belongs in a function's body"));
        };
        if !matches!(info.parts.as_slice(), [NamePart::Group, NamePart::Sym(op), NamePart::Group] if op == "<<") {
            return Err(lex::error(&file, line, format!("`existing x$ << item` is for a `<<` method; this is '{}'", spoken(&info))));
        }
        if info.params[0].0 != name {
            return Err(lex::error(&file, line, format!("'{}' is not this method's stream: `existing {}$ << item`", name, info.params[0].0)));
        }
        let Some(below) = b.below.clone() else {
            return Err(lex::error(&file, line, format!("no earlier definition of '{}' for 'existing' to call{}", spoken(&info), if info.chain.len() > 1 { ": this is the first" } else { "" })));
        };
        let mut v = self.lower_expr(item, Some(&info.params[1].1), b, None)?;
        if v.literal && fits_literal(&v, &info.params[1].1) {
            v.ty = info.params[1].1.clone();
        }
        let v = b.materialize(&v);
        if self.device_param.as_deref() == Some(name) {
            b.line(&format!("{}({})", below, v.text));
        } else {
            let s = self.lower_expr(&Expr { kind: ExprKind::Name(name.to_string()), line }, None, b, None)?;
            b.line(&format!("{}({}, {})", below, s.text, v.text));
        }
        Ok(())
    }

    /// `q, r = f(...)`: a call with several results defines several variables
    fn lower_multi(&mut self, value: &Expr, names: &[String], tys: &[Ty], b: &mut Body, line: usize) -> Result<(), Error> {
        let file = b.file.clone();
        if let ExprKind::Existing(parts) = &value.kind {
            let (call, rtys) = self.existing_call(parts, b, line)?;
            if rtys.len() != names.len() {
                return Err(lex::error(&file, line, format!("'existing' gives {} result(s), {} wanted", rtys.len(), names.len())));
            }
            for ((n, want), got) in names.iter().zip(tys).zip(&rtys) {
                if want != got {
                    return Err(lex::error(&file, line, format!("'{}' is {} but 'existing' gives {}", n, want.ir(), got.ir())));
                }
            }
            let mut sets = Vec::new();
            let defs: Vec<String> = names
                .iter()
                .zip(tys)
                .map(|(n, t)| {
                    if b.vars.contains_key(n) {
                        format!("{}: {}", b.define(n, t.clone()), t.ir())
                    } else {
                        let tmp = b.tmp();
                        sets.push((n.clone(), tmp.clone()));
                        format!("{}: {}", tmp, t.ir())
                    }
                })
                .collect();
            b.line(&format!("{} = {}", defs.join(", "), call));
            for (n, tmp) in sets {
                self.field_put(&n, &tmp, b);
            }
            return Ok(());
        }
        let ExprKind::Phrase(parts) = &value.kind else {
            return Err(lex::error(&file, line, "several variables at once take a call with several results"));
        };
        // `position x$` gives the index alone now (log 85, question 42)
        if let [Part::Word(w), Part::Value(Expr { kind: ExprKind::Seq(n), .. })] = parts.as_slice() {
            if w == "position" && self.stream_var(n, b).is_some() {
                return Err(lex::error(&file, line, format!("'position' gives one int, the index of the next unread item: `int i = position {}$`, and the time of that item is `time of {}$`", n, n)));
            }
        }
        let is_var = |w: &str| b.vars.contains_key(w) || self.fvar(w).is_some();
        let (cands, args) = find_methods(&self.funcs, parts, &is_var, &file, line)?;
        let cands: Vec<FnInfo> = cands.into_iter().cloned().collect();
        if cands[0].task {
            return Err(lex::error(&file, line, task_refusal(&cands[0], value)));
        }
        let (info, (vals, lifted, acc, rtys, _)) = self.choose(&cands, &args, b, line)?;
        if lifted.iter().any(|&l| l) || acc.is_some() {
            return Err(lex::error(&file, line, format!("'{}' gives several results: it is not mapped over a stream", info.key)));
        }
        self.reach(&spoken(&info), &info.feature, &file, line)?;
        if info.results.len() != names.len() {
            return Err(lex::error(&file, line, format!("'{}' gives {} result(s), {} wanted", info.key, info.results.len(), names.len())));
        }
        let ops: Vec<String> = vals.into_iter().map(|v| v.text).collect();
        for ((n, want), got) in names.iter().zip(tys).zip(&rtys) {
            if want != got && !widens(got, want) {
                return Err(lex::error(&file, line, format!("'{}' is {} but '{}' gives {}", n, want.ir(), info.key, got.ir())));
            }
        }
        // a feature variable among the targets takes its result through
        // a temporary and a store of its field; a narrower result widens through
        // a temporary and a `conv` (log 37)
        let mut sets = Vec::new();
        let mut convs = Vec::new();
        let defs: Vec<String> = names
            .iter()
            .zip(tys)
            .zip(&rtys)
            .map(|((n, t), got)| {
                if got != t {
                    let tmp = b.tmp();
                    convs.push((n.clone(), tmp.clone(), got.clone()));
                    format!("{}: {}", tmp, got.ir())
                } else if b.vars.contains_key(n) {
                    format!("{}: {}", b.define(n, t.clone()), t.ir())
                } else {
                    let tmp = b.tmp();
                    sets.push((n.clone(), tmp.clone()));
                    format!("{}: {}", tmp, t.ir())
                }
            })
            .collect();
        b.line(&format!("{} = {}({})", defs.join(", "), info.ir, ops.join(", ")));
        for (n, tmp, got) in convs {
            let v = Val { text: tmp, ty: got, literal: false };
            if b.vars.contains_key(&n) {
                let ty = b.vars[&n].ty.clone();
                let w = self.widen_to(&v, &ty, b, Some(&n));
                self.assign(&n, w, b, line)?;
            } else {
                self.write_fvar(&n, v, b, line)?;
            }
        }
        for (n, tmp) in sets {
            self.field_put(&n, &tmp, b);
        }
        Ok(())
    }

    /// a call's arguments lowered against its parameters, and its result
    /// types with the tower's abstract names bound by the arguments; an
    /// argument that is a sequence where the parameter is an item is
    /// marked lifted (a map), and `_` marks the accumulator (a reduce)
    fn lower_call_args(&mut self, info: &FnInfo, args: &[Expr], b: &mut Body) -> Result<(Vec<Val>, Vec<bool>, Option<(usize, Ty)>, Vec<Ty>, bool), Error> {
        let file = b.file.clone();
        // the round is this call's alone: a call inside an argument
        // chooses for itself
        let round = std::mem::replace(&mut self.round, Round::Any);
        let mut vals = Vec::new();
        let mut lifted = Vec::new();
        let mut acc = None;
        let mut bound: HashMap<String, Ty> = HashMap::new();
        // the arguments that bound an abstract name: index, name, lifted
        let mut binders: Vec<(usize, String, bool)> = Vec::new();
        // did any argument widen? `choose` prefers a method where none did
        let mut converted = false;
        for (i, (a, (_, ty))) in args.iter().zip(&info.params).enumerate() {
            if matches!(a.kind, ExprKind::Acc) && self.candidate.is_none() {
                if acc.is_some() {
                    return Err(lex::error(&file, a.line, "one '_' marks the accumulator"));
                }
                acc = Some(i);
                vals.push(Val { text: "_".into(), ty: ty.clone(), literal: false });
                lifted.push(false);
                continue;
            }
            let mut v = self.lower_expr(a, Some(ty), b, None)?;
            if round != Round::Any {
                // a literal, or a list of them, as its own type — or as
                // the width being tried (log 52) — while `choose` asks
                let (int, float) = if round == Round::Product { self.trial.clone() } else { (int_ty(), float_ty()) };
                if let Some(d) = literal_default(a, &int, &float) {
                    if !fits(&d, ty) {
                        return Err(lex::error(&file, a.line, format!("'{}' wants a {} here, given an {}", info.key, ty.ir(), d.ir())));
                    }
                }
            }
            if v.literal && fits_literal(&v, ty) {
                v.ty = ty.clone();
            }
            // a literal meeting an abstract parameter binds it to the
            // literal's own type, `int` or `float`, as a typed constant:
            // the IR cannot type `2.5` under `number` on its own
            if v.literal && ty.abstract_name().is_some() {
                if let Some(d) = literal_default(a, &int_ty(), &float_ty()) {
                    if &d != ty && fits(&d, ty) {
                        v.ty = d;
                        v = b.materialize(&v);
                    }
                }
            }
            // a literal to the first method of a set, called by the plain
            // name, says its type: the IR's set resolves by it (log 52)
            if v.literal && info.in_set && info.ir == info.key {
                v.text = format!("{}: {}", v.text, v.ty.ir());
            }
            // a stream where an item is wanted is mapped over: the
            // item's type is what the tower sees
            let item = match (v.ty.items(), ty) {
                (Some(_), Ty::Stream(_)) => v.ty.clone(),
                (Some(e), _) => {
                    lifted.push(true);
                    e.clone()
                }
                _ => v.ty.clone(),
            };
            if lifted.len() < vals.len() + 1 {
                lifted.push(false);
            }
            let lifted_here = *lifted.last().unwrap();
            if !fits(&item, ty) {
                // a narrower number widens to a concrete parameter (log 37)
                if !lifted_here && widens(&item, ty) {
                    v = self.widen_to(&v, ty, b, None);
                    converted = true;
                } else {
                    return Err(lex::error(&file, a.line, format!("'{}' wants a {} here, given a {}", info.key, ty.ir(), v.ty.ir())));
                }
            }
            // an abstract element binds through the stream (log 59)
            let (item, ty) = match (&item, ty) {
                (Ty::Stream(a), Ty::Stream(p)) if p.abstract_name().is_some() => (a.as_ref().clone(), p.as_ref()),
                _ => (item, ty),
            };
            if let Some(name) = ty.abstract_name() {
                if &item != ty {
                    // the substitution: each abstract name binds to the
                    // widest of the types its arguments bring (log 37)
                    let prev = bound.get(name).cloned();
                    let joined = match &prev {
                        None => item.clone(),
                        Some(p) => match wider(p, &item) {
                            Some(w) => w,
                            None => return Err(lex::error(&file, a.line, format!("'{}' takes {} at two places, given a {} and a {}: no number type computes both; convert one", info.key, name, zero_ty(p), zero_ty(&item)))),
                        },
                    };
                    bound.insert(name.to_string(), joined);
                    binders.push((vals.len(), name.to_string(), lifted_here));
                }
            }
            vals.push(v);
        }
        // the arguments narrower than what their name bound to widen
        for (i, name, lifted_here) in binders {
            let to = bound[&name].clone();
            let whole = matches!(vals[i].ty, Ty::Stream(_));
            let have = if lifted_here || whole { vals[i].ty.elem().cloned().unwrap() } else { vals[i].ty.clone() };
            if have != to {
                if lifted_here || whole {
                    return Err(lex::error(&file, args[i].line, format!("'{}' binds {} to {} here, and a sequence of {} is not widened item by item", info.key, name, zero_ty(&to), zero_ty(&vals[i].ty))));
                }
                vals[i] = self.widen_to(&vals[i].clone(), &to, b, None);
                converted = true;
            }
        }
        let resolve = |t: &Ty| match t.abstract_name().and_then(|n| bound.get(n)) {
            Some(bt) => bt.clone(),
            None => t.clone(),
        };
        let rtys = info.results.iter().map(|(_, t)| resolve(t)).collect();
        let acc = acc.map(|i| {
            let aty = resolve(&info.params[i].1);
            vals[i].ty = aty.clone();
            (i, aty)
        });
        Ok((vals, lifted, acc, rtys, converted))
    }

    /// The method a call takes (section 6, log 36). Each candidate is
    /// tried on the arguments — a try that fails leaves nothing behind —
    /// first with every literal as its own type (`3` an `int`, `2.5` a
    /// `float`), then, when no method takes that, with a literal fitting
    /// any number type; a method that takes an argument as it is beats
    /// one the call would have to map over a sequence; among what is
    /// left `pick` decides. The winner's arguments are then lowered for
    /// good
    fn choose(&mut self, cands: &[FnInfo], args: &[Expr], b: &mut Body, line: usize) -> Result<(FnInfo, (Vec<Val>, Vec<bool>, Option<(usize, Ty)>, Vec<Ty>, bool)), Error> {
        let file = b.file.clone();
        if let [one] = cands {
            let r = self.lower_call_args(one, args, b)?;
            return Ok((one.clone(), r));
        }
        let mut last_err = None;
        // a literal as its own type
        let own = self.applicable(cands, args, b, Round::Own, (int_ty(), float_ty()), &mut last_err);
        if !own.is_empty() {
            let i = pick(cands, &own).map_err(|amb| ambiguous(cands, &amb, &file, line))?;
            let r = self.lower_call_args(&cands[i], args, b)?;
            return Ok((cands[i].clone(), r));
        }
        // a literal as a concrete width the policy may take, each tried
        // (log 52): one method for every width is simply chosen; several
        // are emitted on the name for the policy to choose among. The
        // widths are the policy's two, whatever `product.md` pins: the
        // pin reaches the policy, never the text
        let mut picks: Vec<usize> = Vec::new();
        for iw in WIDTHS {
            for fw in WIDTHS {
                let trial = (Ty::Num(format!("i{}", iw)), Ty::Num(format!("f{}", fw)));
                let app = self.applicable(cands, args, b, Round::Product, trial, &mut last_err);
                if app.is_empty() {
                    picks.clear();
                    break;
                }
                let i = pick(cands, &app).map_err(|amb| ambiguous(cands, &amb, &file, line))?;
                if !picks.contains(&i) {
                    picks.push(i);
                }
            }
        }
        match picks.as_slice() {
            [] => {}
            [i] => {
                let r = self.lower_call_args(&cands[*i], args, b)?;
                return Ok((cands[*i].clone(), r));
            }
            _ => return self.policy_call(cands, &picks, args, b, line),
        }
        // a literal fitting any number type
        let any = self.applicable(cands, args, b, Round::Any, (int_ty(), float_ty()), &mut last_err);
        if !any.is_empty() {
            let i = pick(cands, &any).map_err(|amb| ambiguous(cands, &amb, &file, line))?;
            let r = self.lower_call_args(&cands[i], args, b)?;
            return Ok((cands[i].clone(), r));
        }
        let sigs: Vec<String> = cands.iter().map(spelled).collect();
        let err = last_err.unwrap();
        Err(lex::error(&file, err.line, format!("no '{}' takes these arguments: the methods are {}", spoken(&cands[0]), sigs.join(", "))))
    }

    /// the methods that take the arguments in one round of literal
    /// typing, each tried and rolled back: taken as they are, before
    /// widened (log 37), before mapped
    fn applicable(&mut self, cands: &[FnInfo], args: &[Expr], b: &mut Body, round: Round, trial: (Ty, Ty), last_err: &mut Option<Error>) -> Vec<usize> {
        let mut direct = Vec::new();
        let mut widened = Vec::new();
        let mut mapped = Vec::new();
        for (i, cand) in cands.iter().enumerate() {
            let (start, ntmp, vars, defs, ndata, nstr) = (b.out.len(), b.ntmp, b.vars.clone(), b.defs.clone(), self.data.len(), self.nstr);
            self.round = round;
            self.trial = trial.clone();
            let tried = self.lower_call_args(cand, args, b);
            self.round = Round::Any;
            match tried {
                Ok((_, lifted, acc, _, _)) if lifted.iter().any(|&l| l) || acc.is_some() => mapped.push(i),
                Ok((_, _, _, _, true)) => widened.push(i),
                Ok(_) => direct.push(i),
                Err(err) => *last_err = Some(err),
            }
            b.out.truncate(start);
            b.ntmp = ntmp;
            b.vars = vars;
            b.defs = defs;
            self.data.truncate(ndata);
            self.nstr = nstr;
        }
        [direct, widened, mapped].into_iter().find(|g| !g.is_empty()).unwrap_or_default()
    }

    /// a call the policy decides (question 27, log 52): the widths the
    /// policy may take chose different methods, so the call is emitted
    /// on the plain name, which is a method set in the IR, with each
    /// literal that told them apart typed `int` or `float` — the
    /// policy's — and the IR picks the method by its resolved type. The
    /// methods must give the same results and differ only at such
    /// literals, or the call is ambiguous and a conversion says which
    fn policy_call(&mut self, cands: &[FnInfo], picks: &[usize], args: &[Expr], b: &mut Body, line: usize) -> Result<(FnInfo, (Vec<Val>, Vec<bool>, Option<(usize, Ty)>, Vec<Ty>, bool)), Error> {
        let file = b.file.clone();
        let first = &cands[picks[0]];
        let rtys = |g: &FnInfo| g.results.iter().map(|(_, t)| t.clone()).collect::<Vec<Ty>>();
        let names: Vec<String> = picks.iter().map(|&i| spelled(&cands[i])).collect();
        if !first.in_set {
            return Err(lex::error(&file, line, format!("ambiguous: {} all take these arguments and the policy's width would choose, but a method of the name is over an abstract type, so the IR cannot hold them as one set; convert the literal", names.join(" and "))));
        }
        if picks.iter().any(|&i| rtys(&cands[i]) != rtys(first)) {
            return Err(lex::error(&file, line, format!("ambiguous: the policy's width would choose among {}, which give different results; convert the literal", names.join(" and "))));
        }
        let (mut vals, lifted, acc, rtys, converted) = self.lower_call_args(first, args, b)?;
        for k in 0..first.params.len() {
            let tys: Vec<&Ty> = picks.iter().map(|&i| &cands[i].params[k].1).collect();
            if tys.iter().all(|t| *t == tys[0]) {
                continue;
            }
            let literal = literal_default(&args[k], &int_ty(), &float_ty()).filter(|_| vals[k].literal);
            let family = match literal {
                Some(Ty::Num(n)) if n == "int" => "int",
                Some(Ty::Num(n)) if n == "float" => "float",
                _ => return Err(lex::error(&file, line, format!("ambiguous: {} all take these arguments and the policy's width would choose; convert the argument that differs", names.join(" and ")))),
            };
            if !tys.iter().all(|t| fits(t, &Ty::Num(family.into())) && t.abstract_name().is_none()) {
                return Err(lex::error(&file, line, format!("ambiguous: {} all take these arguments and none is the most specific; convert the literal", names.join(" and "))));
            }
            let text = vals[k].text.split(':').next().unwrap().trim().to_string();
            vals[k] = Val { text: format!("{}: {}", text, family), ty: Ty::Num(family.into()), literal: false };
        }
        Ok((FnInfo { ir: first.plain.clone(), ..first.clone() }, (vals, lifted, acc, rtys, converted)))
    }

    /// a call's arguments where no sequence is lifted: the operand texts
    fn lower_args(&mut self, info: &FnInfo, args: &[Expr], b: &mut Body) -> Result<(Vec<String>, Vec<Ty>), Error> {
        let file = b.file.clone();
        let (vals, lifted, acc, rtys, _) = self.lower_call_args(info, args, b)?;
        if lifted.iter().any(|&l| l) || acc.is_some() {
            return Err(lex::error(&file, args[0].line, format!("'{}' gives several results: it is not mapped over a sequence", info.key)));
        }
        Ok((vals.into_iter().map(|v| v.text).collect(), rtys))
    }

    /// Map, zip and reduce (log 19, 38): `vals` are an operation's
    /// operands, those marked lifted being streams whose unread items
    /// the operation takes one at a time; `op` emits the operation on
    /// one set of items. With no accumulator the results make a new
    /// stream, as long as the longest input, a shorter one reading as
    /// zero past its end; with one, the operation folds over the one
    /// stream, from its first item, an empty one giving the
    /// accumulator's zero.
    fn lift(&mut self, mut vals: Vec<Val>, lifted: Vec<bool>, acc: Option<(usize, Ty)>, b: &mut Body, dst: Option<&str>, line: usize, op: &dyn Fn(&mut Lowerer, &[Val], &mut Body) -> Result<Val, Error>) -> Result<Val, Error> {
        let file = b.file.clone();
        let seqs: Vec<usize> = (0..vals.len()).filter(|&i| lifted[i]).collect();
        if acc.is_some() && seqs.len() != 1 {
            return Err(lex::error(&file, line, "a reduction folds one stream"));
        }
        let mut lens = Vec::new();
        for &i in &seqs {
            if vals[i].ty.items().is_none() {
                return Err(lex::error(&file, line, format!("a map over a {}: a stream of structs is not mapped, zipped or reduced in this milestone", zero_ty(&vals[i].ty))));
            }
            let view = self.unread_view(&vals[i], b);
            let n = b.tmp();
            b.line(&format!("{}: i64 = len {}", n, view));
            lens.push(n);
            vals[i].text = view;
        }
        let mut n = lens[0].clone();
        for l in &lens[1..] {
            let m = b.tmp();
            b.line(&format!("{}: i64 = max({}, {})", m, n, l));
            n = m;
        }
        let k = b.tmp();
        let zip = seqs.len() > 1;
        // the items of this pass, a shorter sequence's zero past its end
        let load_items = |b: &mut Body, vals: &mut Vec<Val>, from: &str| {
            for (j, &i) in seqs.iter().enumerate() {
                let e = vals[i].ty.elem().unwrap().clone();
                let x = b.tmp();
                if zip {
                    let inside = b.tmp();
                    b.line(&format!("{}: u1 = cmp.lt {}, {}", inside, from, lens[j]));
                    b.line(&format!("{}: {} = if {}", x, e.ir(), inside));
                    b.depth += 1;
                    let y = b.tmp();
                    b.line(&format!("{}: {} = load {}, {}", y, e.ir(), vals[i].text, from));
                    b.line(&format!("yield {}", y));
                    b.depth -= 1;
                    b.line("else");
                    b.depth += 1;
                    b.line("yield 0");
                    b.depth -= 1;
                } else {
                    b.line(&format!("{}: {} = load {}, {}", x, e.ir(), vals[i].text, from));
                }
                vals[i] = Val { text: x, ty: e, literal: false };
            }
        };
        match acc {
            None => {
                let c = b.tmp();
                let start = b.out.len();
                b.depth += 1;
                let done = b.tmp();
                b.line(&format!("{}: u1 = cmp.ge {}, {}", done, k, n));
                b.line(&format!("if {}", done));
                b.depth += 1;
                b.line("break");
                b.depth -= 1;
                load_items(b, &mut vals, &k);
                let r = op(self, &vals, b)?;
                if r.ty == Ty::None {
                    // a function with no result over a stream: one call per
                    // item and nothing made (log 40)
                    let k2 = b.tmp();
                    b.line(&format!("{}: i64 = add {}, 1", k2, k));
                    b.line(&format!("continue {}", k2));
                    b.depth -= 1;
                    let body = b.out.split_off(start);
                    b.open_loop("", &format!("{}: i64 = 0", k), false);
                    b.out.push_str(&body);
                    return Ok(r);
                }
                let r = b.materialize(&r);
                let maker = self.flavour(dst, b);
                let t = b.tmp();
                if maker == "queue" {
                    let word = self.queue_push(Some(&r.ty));
                    b.line(&format!("{}({}, {})", word, c, r.text));
                } else if maker == "regular" {
                    b.line(&format!("push {}, {}", c, r.text));
                } else {
                    b.line(&format!("push {}, {}, {}", c, t, r.text));
                }
                let k2 = b.tmp();
                b.line(&format!("{}: i64 = add {}, 1", k2, k));
                b.line(&format!("continue {}", k2));
                b.depth -= 1;
                let body = b.out.split_off(start);
                if !matches!(r.ty, Ty::Num(_) | Ty::Enum(_)) {
                    return Err(lex::error(&file, line, format!("a map giving a {}: a stream holds numbers and enumerations", zero_ty(&r.ty))));
                }
                // the results' ring, before the loop: as many items as the
                // longest input, stamped once
                let rty = Ty::Stream(Box::new(r.ty.clone()));
                self.rings.insert((r.ty.ir(), maker.to_string()));
                let least = b.tmp();
                b.line(&format!("{}: i64 = const {}", least, RING_ITEMS));
                let cap = b.tmp();
                b.line(&format!("{}: i64 = max({}, {})", cap, n, least));
                b.line(&format!("{}: {} = __{}_{}({}, {})", c, rty.ir(), maker, r.ty.ir(), CLOCK_HZ, cap));
                if maker == "stream" {
                    b.line(&format!("{}: i64 = __now()", t));
                }
                b.open_loop("", &format!("{}: i64 = 0", k), false);
                b.out.push_str(&body);
                let _ = dst;
                Ok(Val { text: c, ty: rty, literal: false })
            }
            Some((ai, aty)) => {
                let si = seqs[0];
                let e = vals[si].ty.elem().unwrap().clone();
                let v = vals[si].text.clone();
                let empty = b.tmp();
                b.line(&format!("{}: u1 = cmp.eq {}, 0", empty, n));
                let seed = b.tmp();
                b.line(&format!("{}: {} = if {}", seed, aty.ir(), empty));
                b.depth += 1;
                b.line("yield 0");
                b.depth -= 1;
                b.line("else");
                b.depth += 1;
                let first = b.tmp();
                b.line(&format!("{}: {} = load {}, 0", first, e.ir(), v));
                if e != aty {
                    return Err(lex::error(&file, line, format!("the accumulator is a {} but the items are {}", aty.ir(), e.ir())));
                }
                b.line(&format!("yield {}", first));
                b.depth -= 1;
                let a = b.tmp();
                let start = b.out.len();
                b.depth += 1;
                let done = b.tmp();
                b.line(&format!("{}: u1 = cmp.ge {}, {}", done, k, n));
                b.line(&format!("if {}", done));
                b.depth += 1;
                b.line(&format!("break {}", a));
                b.depth -= 1;
                load_items(b, &mut vals, &k);
                vals[ai] = Val { text: a.clone(), ty: aty.clone(), literal: false };
                let r = op(self, &vals, b)?;
                let r = b.materialize(&r);
                if r.ty != aty {
                    return Err(lex::error(&file, line, format!("the reduction gives a {} but its accumulator is a {}", r.ty.ir(), aty.ir())));
                }
                let k2 = b.tmp();
                b.line(&format!("{}: i64 = add {}, 1", k2, k));
                b.line(&format!("continue {}, {}", k2, r.text));
                b.depth -= 1;
                let body = b.out.split_off(start);
                let out = name_for(dst, &aty, b);
                b.open_loop(&format!("{}: {} = ", out, aty.ir()), &format!("{}: i64 = 1, {}: {} = {}", k, a, aty.ir(), seed), false);
                b.out.push_str(&body);
                Ok(Val { text: out, ty: aty, literal: false })
            }
        }
    }

    /// `[a, b, c]`: a new stream with the items resident
    fn lower_list(&mut self, items: &[Expr], want: Option<&Ty>, b: &mut Body, dst: Option<&str>, line: usize) -> Result<Val, Error> {
        let file = b.file.clone();
        let want_e = want.and_then(|t| t.elem()).cloned();
        let mut vals = Vec::new();
        for it in items {
            vals.push(self.lower_expr(it, want_e.as_ref(), b, None)?);
        }
        let e = want_e.or_else(|| vals.iter().find(|v| !v.literal).map(|v| v.ty.clone())).or_else(|| vals.first().map(|v| v.ty.clone()));
        let Some(e) = e else {
            return Err(lex::error(&file, line, "an empty list needs a type: declare the stream, `int i$`"));
        };
        if !matches!(e, Ty::Num(_) | Ty::Enum(_)) {
            return Err(lex::error(&file, line, format!("a list of {}: only numbers and enumerations in this milestone", zero_ty(&e))));
        }
        for (it, v) in items.iter().zip(&vals) {
            if !(v.ty == e || (v.literal && fits_literal(v, &e))) {
                return Err(lex::error(&file, it.line, format!("the items are {}, this one is a {}", e.ir(), v.ty.ir())));
            }
        }
        let (c, t) = self.new_resident(&e, &vals.len().to_string(), b, dst);
        for v in &vals {
            // a literal pushed after a stream takes the item's type
            match &t {
                Some(t) => b.line(&format!("push {}, {}, {}", c.text, t, v.text)),
                None => b.line(&format!("push {}, {}", c.text, v.text)),
            }
        }
        Ok(c)
    }

    /// `[a through b]`, `[a to b]`: a new stream with the items resident,
    /// pushed by a loop that counts down when a > b — with literal
    /// bounds, the plain counted loop `probe cost` reads (log 13, 38);
    /// or, pushed as a block, `x$ << [a through b]`, the same loop
    /// pushing each value straight into `x$` (log 41)
    fn lower_range(&mut self, from: &Expr, to: &Expr, inclusive: bool, b: &mut Body, sink: RangeSink, line: usize) -> Result<Val, Error> {
        let file = b.file.clone();
        for e in [from, to] {
            if matches!(e.kind, ExprKind::Float(_)) {
                return Err(lex::error(&file, e.line, "a range's bounds are integers"));
            }
        }
        let mut fv = self.lower_expr(from, None, b, None)?;
        let mut tv = self.lower_expr(to, if fv.literal { None } else { Some(&fv.ty) }, b, None)?;
        if fv.literal && !tv.literal {
            fv.ty = tv.ty.clone();
        }
        if tv.literal && !fv.literal {
            tv.ty = fv.ty.clone();
        }
        let ty = fv.ty.clone();
        let signed = matches!(&ty, Ty::Num(n) if n == "int" || (n.starts_with('i') && n[1..].parse::<u32>().is_ok()));
        if !signed || ty != tv.ty {
            return Err(lex::error(&file, line, format!("a range counts over a signed integer, given {} and {}", fv.ty.ir(), tv.ty.ir())));
        }
        if let RangeSink::Into(name, s) = sink {
            if s.ty.elem() != Some(&ty) {
                return Err(lex::error(&file, line, format!("'{}$' holds {} but the range counts over {}", name, s.ty.elem().map(|t| t.ir()).unwrap_or_default(), ty.ir())));
            }
        }
        // where each value goes: a new ring, stamped once, or the stream
        let target = |l: &mut Lowerer, count: &str, b: &mut Body| -> (Val, Option<Option<String>>) {
            match sink {
                RangeSink::New(dst) => {
                    let (c, t) = l.new_resident(&ty, count, b, dst);
                    (c, Some(t))
                }
                RangeSink::Into(_, s) => (s.clone(), None),
            }
        };
        let emit = |l: &mut Lowerer, c: &Val, t: &Option<Option<String>>, x: &str, b: &mut Body| match (t, sink) {
            (Some(Some(t)), _) => b.line(&format!("push {}, {}, {}", c.text, t, x)),
            (Some(None), _) => b.line(&format!("push {}, {}", c.text, x)),
            (None, RangeSink::Into(name, _)) => l.emit_push(name, c, &Val { text: x.to_string(), ty: ty.clone(), literal: false }, b),
            _ => unreachable!(),
        };
        if fv.literal && tv.literal {
            let a: i64 = fv.text.parse().map_err(|_| lex::error(&file, from.line, "a range's bounds are integers"))?;
            let z: i64 = tv.text.parse().map_err(|_| lex::error(&file, to.line, "a range's bounds are integers"))?;
            let up = a <= z;
            let cc = match (up, inclusive) {
                (true, true) => "cmp.le",
                (true, false) => "cmp.lt",
                (false, true) => "cmp.ge",
                (false, false) => "cmp.gt",
            };
            let count = (a - z).abs() + inclusive as i64;
            let (c, t) = target(self, &count.to_string(), b);
            let x = b.tmp();
            b.open_loop("", &format!("{}: {} = {}", x, ty.ir(), a), true);
            b.depth += 1;
            let more = b.tmp();
            b.line(&format!("{}: u1 = {} {}, {}", more, cc, x, z));
            b.line(&format!("if {}", more));
            b.line("else");
            b.depth += 1;
            b.line("break");
            b.depth -= 1;
            emit(self, &c, &t, &x, b);
            let x2 = b.tmp();
            b.line(&format!("{}: {} = {} {}, 1", x2, ty.ir(), if up { "add" } else { "sub" }, x));
            b.line(&format!("continue {}", x2));
            b.depth -= 1;
            return Ok(c);
        }
        let fv = b.materialize(&fv);
        let tv = b.materialize(&tv);
        let d = b.tmp();
        b.line(&format!("{}: {} = sub {}, {}", d, ty.ir(), tv.text, fv.text));
        let down = b.tmp();
        b.line(&format!("{}: u1 = cmp.lt {}, 0", down, d));
        let step = b.tmp();
        b.line(&format!("{}: {} = if {}", step, ty.ir(), down));
        b.depth += 1;
        b.line("yield -1");
        b.depth -= 1;
        b.line("else");
        b.depth += 1;
        b.line("yield 1");
        b.depth -= 1;
        let span = b.tmp();
        b.line(&format!("{}: {} = mul {}, {}", span, ty.ir(), d, step));
        let count = if inclusive {
            let c = b.tmp();
            b.line(&format!("{}: {} = add {}, 1", c, ty.ir(), span));
            c
        } else {
            span
        };
        let n = b.tmp();
        b.line(&format!("{}: i64 = conv {}", n, count));
        let (c, t) = target(self, &n, b);
        let k = b.tmp();
        let x = b.tmp();
        b.open_loop("", &format!("{}: i64 = 0, {}: {} = {}", k, x, ty.ir(), fv.text), false);
        b.depth += 1;
        let done = b.tmp();
        b.line(&format!("{}: u1 = cmp.ge {}, {}", done, k, n));
        b.line(&format!("if {}", done));
        b.depth += 1;
        b.line("break");
        b.depth -= 1;
        emit(self, &c, &t, &x, b);
        let k2 = b.tmp();
        b.line(&format!("{}: i64 = add {}, 1", k2, k));
        let x2 = b.tmp();
        b.line(&format!("{}: {} = add {}, {}", x2, ty.ir(), x, step));
        b.line(&format!("continue {}, {}", k2, x2));
        b.depth -= 1;
        Ok(c)
    }

    /// arithmetic or a comparison on two scalars: a literal takes the
    /// other side's type, two literals make one a value first
    /// An operator on two numbers (log 7, 37). A literal takes the other
    /// side's type. Two concrete types compute in the wider of them,
    /// the narrower converted; then, when the expression's wanted type
    /// is a concrete number both widen to exactly, in that — the result
    /// type drives the conversion, so `float64 q = a / b` on two
    /// `int32`s divides as floats. A comparison gives a bool
    fn emit_bin(&mut self, op: &str, mut lv: Val, mut rv: Val, want: Option<&Ty>, b: &mut Body, dst: Option<&str>, line: usize) -> Result<Val, Error> {
        let file = b.file.clone();
        let cmp = is_comparison(op);
        if lv.literal && !rv.literal {
            if !fits_literal(&lv, &rv.ty) {
                return Err(lex::error(&file, line, format!("'{}' on a decimal and a {}", op, zero_ty(&rv.ty))));
            }
            lv.ty = rv.ty.clone();
        }
        if rv.literal && !lv.literal {
            if !fits_literal(&rv, &lv.ty) {
                return Err(lex::error(&file, line, format!("'{}' on a {} and a decimal", op, zero_ty(&lv.ty))));
            }
            rv.ty = lv.ty.clone();
        }
        if lv.literal && rv.literal {
            // two literals: the decimal one, if either, says the type
            if rv.text.contains('.') && !lv.text.contains('.') {
                lv.ty = rv.ty.clone();
            }
            lv = b.materialize(&lv);
            rv.ty = lv.ty.clone();
        }
        if lv.ty != rv.ty {
            let Some(common) = wider(&lv.ty, &rv.ty) else {
                return Err(lex::error(&file, line, format!("'{}' on a {} and a {}: no number type computes both; convert one, {}(x)", op, zero_ty(&lv.ty), zero_ty(&rv.ty), zero_ty(&rv.ty))));
            };
            lv = self.widen_to(&lv, &common, b, None);
            rv = self.widen_to(&rv, &common, b, None);
        }
        if !cmp {
            if let Some(w @ Ty::Num(_)) = want {
                if widens(&lv.ty, w) {
                    lv = self.widen_to(&lv, w, b, None);
                    rv = self.widen_to(&rv, w, b, None);
                }
            }
        }
        let equality = cmp && matches!(op, "==" | "!=");
        match &lv.ty {
            Ty::Num(_) => {}
            // a char is a character, not a small number (question 44):
            // it is ordered, so the lexer may write `c <= 32`, and it
            // is not added to
            Ty::Char if cmp => {}
            Ty::Char => return Err(lex::error(&file, line, format!("'{}' on a char: a char is compared, not computed with; convert it, `int(c)`", op))),
            Ty::Bool | Ty::Enum(_) if equality => {}
            Ty::Enum(_) => return Err(lex::error(&file, line, format!("'{}' on an enumeration: only '==' and '!=' apply", op))),
            t => return Err(lex::error(&file, line, format!("'{}' takes numbers, not a {}", op, t.ir()))),
        }
        let ty = if cmp { Ty::Bool } else { lv.ty.clone() };
        let name = name_for(dst, &ty, b);
        b.line(&format!("{}: {} = {} {}, {}", name, ty.ir(), op_name(op), lv.text, rv.text));
        Ok(Val { text: name, ty, literal: false })
    }

    /// an operator with a stream on a side: a map or a zip over the
    /// unread items, a loop of pushes into a new stream (log 38)
    fn seq_bin(&mut self, op: &str, lv: Val, rv: Val, b: &mut Body, dst: Option<&str>, line: usize) -> Result<Val, Error> {
        let file = b.file.clone();
        if is_comparison(op) {
            return Err(lex::error(&file, line, "a comparison over a stream is not in this milestone"));
        }
        let lifted = vec![lv.ty.elem().is_some(), rv.ty.elem().is_some()];
        let op = op.to_string();
        let f = move |s: &mut Lowerer, ev: &[Val], b: &mut Body| s.emit_bin(&op, ev[0].clone(), ev[1].clone(), None, b, None, line);
        self.lift(vec![lv, rv], lifted, None, b, dst, line, &f)
    }

    /// `x$ + _`, `_ * x$`: a reduction by an operator; `+` is the
    /// library's `sum`
    fn reduce_bin(&mut self, op: &str, l: &Expr, r: &Expr, b: &mut Body, dst: Option<&str>, line: usize) -> Result<Val, Error> {
        let file = b.file.clone();
        let acc_left = matches!(l.kind, ExprKind::Acc);
        let seq = if acc_left { r } else { l };
        let sv = self.lower_expr(seq, None, b, None)?;
        let Some(e) = sv.ty.elem().cloned() else {
            return Err(lex::error(&file, line, "'_' goes with a stream on the other side"));
        };
        if is_comparison(op) || !matches!(e, Ty::Num(_)) {
            return Err(lex::error(&file, line, format!("'{}' does not reduce a stream of {}", op, zero_ty(&e))));
        }
        if op == "+" {
            let view = self.unread_view(&sv, b);
            let name = name_for(dst, &e, b);
            b.line(&format!("{}: {} = sum {}", name, e.ir(), view));
            return Ok(Val { text: name, ty: e, literal: false });
        }
        let acc = Val { text: "_".into(), ty: e.clone(), literal: false };
        let (vals, lifted, ai) = if acc_left { (vec![acc, sv], vec![false, true], 0) } else { (vec![sv, acc], vec![true, false], 1) };
        let op = op.to_string();
        let f = move |s: &mut Lowerer, ev: &[Val], b: &mut Body| s.emit_bin(&op, ev[0].clone(), ev[1].clone(), None, b, None, line);
        self.lift(vals, lifted, Some((ai, e)), b, dst, line, &f)
    }

    /// an enumeration's case by its bare name, when exactly one has it
    fn enum_case(&self, word: &str) -> Option<Val> {
        let mut found = None;
        for (name, info) in &self.types {
            if let TypeInfo::Enum(cases) = info {
                if let Some(i) = cases.iter().position(|c| c == word) {
                    if found.is_some() {
                        return None;
                    }
                    found = Some(Val { text: i.to_string(), ty: Ty::Enum(name.clone()), literal: true });
                }
            }
        }
        found
    }

    /// the program's operator for a struct on the left: among the
    /// methods of the symbol that take the operands, the most specific
    /// (log 36); a literal on the right fits any number
    fn find_operator(&self, op: &str, l: &Ty, r: &Val, file: &str, line: usize) -> Result<Option<FnInfo>, Error> {
        let cands: Vec<FnInfo> = self.funcs.iter().filter(|f| matches!(f.parts.as_slice(), [NamePart::Group, NamePart::Sym(s), NamePart::Group] if s == op)).cloned().collect();
        // a literal on the right is its own type first, `int` or
        // `float`, then any number (an operator symbol has one method
        // per left type, so no width is in question, log 52)
        let decimal = r.text.contains('.');
        // a number literal's own type; a bool's or an enumeration's
        // case is its type as it stands
        let own = if !matches!(r.ty, Ty::Num(_)) { r.ty.clone() } else if decimal { float_ty() } else { int_ty() };
        let takes = |p: &Ty, round: Round| match (r.literal, round) {
            (true, Round::Own) => fits(&own, p),
            (true, _) => fits_literal(r, p),
            (false, _) => fits(&r.ty, p),
        };
        let mut applicable = Vec::new();
        for round in [Round::Own, Round::Any] {
            applicable = (0..cands.len()).filter(|&i| fits(l, &cands[i].params[0].1) && takes(&cands[i].params[1].1, round)).collect();
            if !applicable.is_empty() {
                break;
            }
        }
        if applicable.is_empty() {
            return Ok(None);
        }
        match pick(&cands, &applicable) {
            Ok(i) => Ok(Some(cands[i].clone())),
            Err(amb) => Err(ambiguous(&cands, &amb, file, line)),
        }
    }

    /// Lower an expression to a value. `want` is the type the context
    /// fixes, if any; `dst` a name the result should be defined under.
    // --- streams (section 9, log 23) ---

    /// a declaration's type: a stream for a `$` name (log 38), a plain
    /// value otherwise
    fn decl_ty(&mut self, v: &super::syntax::VarDecl, vars: Option<&HashMap<String, Var>>, file: &str) -> Result<Ty, Error> {
        if !v.seq {
            // a wiring, `T x$ = task(...)`, declares a stream (log 25)
            if let Some(Init::Value(e)) = &v.init {
                if self.task_call(e, vars, file)?.is_some() {
                    return Err(lex::error(file, v.line, format!("a task produces a stream: `{} {}$ = ...`", v.ty, v.name)));
                }
            }
            if v.rate.is_some() {
                return Err(lex::error(file, v.line, "a rate belongs on a stream, `T x$ at (n hz)`"));
            }
        }
        self.ty(&v.ty, v.seq, file, v.line)
    }

    /// a `$` declaration's empty ring: regular at its rate, or on the
    /// clock; `T x$ <<` with nothing after is refused, a bare `T x$`
    /// being the empty stream (question 12)
    fn empty_stream(&mut self, v: &super::syntax::VarDecl, ty: &Ty, b: &mut Body, dst: Option<&str>) -> Result<Val, Error> {
        if let Some(Init::Pushes { items, cond, .. }) = &v.init {
            if items.is_empty() && cond.is_none() {
                return Err(lex::error(&b.file, v.line, format!("a bare `{} {}$` declares an empty stream: drop the `<<`", v.ty, v.name)));
            }
        }
        let hz = match &v.rate {
            Some(r) => self.rate_hz(r, &b.file)?,
            None => CLOCK_HZ,
        };
        // at a rate, or plain because nothing asks its time (log 73):
        // a regular ring; else one that keeps a tick per item
        let maker = if v.rate.is_some() {
            if b.kind == BodyKind::Reset {
                self.rated.insert(v.name.clone());
                self.rates.insert(v.name.clone(), hz);
            }
            // a rate paces the stream; it says nothing about history,
            // so a rated stream nothing keeps is still a queue (log 89)
            let m = self.flavour(Some(&v.name), b);
            if m == "stream" {
                let set = if b.kind == BodyKind::Reset { &mut self.regular } else { &mut self.regular_locals };
                set.insert(v.name.clone());
                "regular"
            } else {
                m
            }
        } else {
            self.flavour(Some(&v.name), b)
        };
        Ok(self.make_stream(ty, hz, maker, b, dst))
    }

    /// `T x$ = e` (log 38): the stream the expression made, its items
    /// resident — a list, a range, a string, a map, a function's result,
    /// or another stream's reader; never at a rate
    fn resident_init(&mut self, v: &super::syntax::VarDecl, ty: &Ty, e: &Expr, b: &mut Body) -> Result<Val, Error> {
        if v.rate.is_some() {
            return Err(lex::error(&b.file, v.line, format!("a rate goes on an empty stream, `{} {}$ at (n hz)`, which `<<` then fills", v.ty, v.name)));
        }
        let val = self.lower_expr(e, Some(ty), b, Some(&v.name))?;
        if val.ty != *ty {
            return Err(lex::error(&b.file, e.line, format!("'{}$' is {} but the value is a {}", v.name, zero_ty(ty), zero_ty(&val.ty))));
        }
        Ok(val)
    }

    /// a stream of an element type: numbers, enumerations, and structs
    /// of those, the last a ring whose item is the struct (log 88,
    /// question 43) — one buffer of `sizeof T` stride, one header, one
    /// position, so a token's push is one push
    fn stream_ty(&mut self, elem: Ty, file: &str, line: usize) -> Result<Ty, Error> {
        match &elem {
            Ty::Num(_) | Ty::Enum(_) | Ty::Char => {}
            Ty::Struct(name) => {
                let Some(TypeInfo::Struct(fields)) = self.types.get(name) else { unreachable!() };
                for (f, t, _) in fields {
                    // the ring stores the struct whole, so a field that
                    // is itself a stream would put a view in a ring
                    if !matches!(t, Ty::Num(_) | Ty::Enum(_) | Ty::Char) {
                        return Err(lex::error(file, line, format!("a stream of {}: field '{}' is a {}, and a stream of structs holds numbers and enumerations in its fields", name, f, zero_ty(t))));
                    }
                }
            }
            _ => return Err(lex::error(file, line, format!("a stream of {}: a stream holds numbers, enumerations or structs of those", zero_ty(&elem)))),
        }
        Ok(Ty::Stream(Box::new(elem)))
    }

    /// `at (n hz)` as a number of hertz
    fn rate_hz(&self, e: &Expr, file: &str) -> Result<i64, Error> {
        if let ExprKind::Unit(inner, u) = &e.kind {
            if let ExprKind::Int(n) = inner.kind {
                match u.as_str() {
                    "hz" if n > 0 => return Ok(n),
                    "khz" if n > 0 => return Ok(n * 1000),
                    _ => {}
                }
            }
        }
        Err(lex::error(file, e.line, "a rate is `at (n hz)` or `at (n khz)`, n positive"))
    }

    /// a new empty stream: its ring in the arena, regular (at a rate) or
    /// with a tick per item, and a reader at its start
    fn make_stream(&mut self, ty: &Ty, hz: i64, maker: &str, b: &mut Body, dst: Option<&str>) -> Val {
        self.make_stream_cap(ty, hz, maker, RING_ITEMS, b, dst)
    }

    /// Is the name the output device (zero.md section 15, question 45)?
    /// The platform feature's own `out$`, not shadowed here — or, inside
    /// the device copy of a `<<` method, that method's stream
    /// parameter. A push into it is the platform's write and stores
    /// nothing, so it has no ring, no field in the context and no node
    fn device(&self, name: &str, b: &Body) -> bool {
        if self.device_param.as_deref() == Some(name) {
            return true;
        }
        if b.vars.contains_key(name) || name != "out" {
            return false;
        }
        self.fvar(name).is_some_and(|v| v.feature == "platform" && matches!(v.ty, Ty::Stream(_)))
    }

    /// Is the name the input device (zero.md section 15, question 35),
    /// named by a program? `in$` is the mirror of `out$`: read and never
    /// written. Input comes from the platform alone, so a program's push
    /// into it, an edge into it and `end in$` are refused. The platform
    /// feature's own code is not a program, and neither is the runner's
    /// `__in_ch`; a local or a parameter named `in` is the function's own
    fn input_device(&self, name: &str, b: Option<&Body>) -> bool {
        name == "in" && self.cur != "platform" && !b.is_some_and(|b| b.vars.contains_key(name)) && self.fvar(name).is_some_and(|v| v.feature == "platform" && matches!(v.ty, Ty::Stream(_)))
    }

    /// the same test on a feature-scope variable alone, for the context
    /// and the accessors, which are emitted before any body
    fn device_var(&self, f: &FVar) -> bool {
        f.name == "out" && f.feature == "platform" && matches!(f.ty, Ty::Stream(_))
    }

    fn make_stream_cap(&mut self, ty: &Ty, hz: i64, maker: &str, cap: usize, b: &mut Body, dst: Option<&str>) -> Val {
        let Ty::Stream(elem) = ty else { unreachable!() };
        self.rings.insert((elem.ir(), maker.to_string()));
        let out = name_for(dst, ty, b);
        b.line(&format!("{}: {} = __{}_{}({}, {})", out, ty.ir(), maker, elem.ir(), hz, cap));
        Val { text: out, ty: ty.clone(), literal: false }
    }

    /// the type of a stream variable in scope, local or feature-scope
    fn stream_var(&self, name: &str, b: &Body) -> Option<Ty> {
        let ty = match b.vars.get(name) {
            Some(v) => v.ty.clone(),
            None => self.fvar(name)?.ty.clone(),
        };
        matches!(ty, Ty::Stream(_)).then_some(ty)
    }

    /// the type of any variable in scope, for a message
    fn seq_or_fvar_ty(&self, name: &str, b: &Body) -> Option<Ty> {
        b.vars.get(name).map(|v| v.ty.clone()).or_else(|| self.fvar(name).map(|f| f.ty.clone()))
    }

    /// an int as the i64 the library takes
    fn as_i64(&mut self, v: &Val, b: &mut Body) -> String {
        if v.literal || v.ty == Ty::Num("i64".into()) {
            return v.text.clone();
        }
        let t = b.tmp();
        b.line(&format!("{}: i64 = conv {}", t, v.text));
        t
    }

    /// the most recent item of a stream
    fn latest_of(&mut self, s: &Val, ty: &Ty, b: &mut Body, dst: Option<&str>) -> Result<Val, Error> {
        let Ty::Stream(elem) = ty else { unreachable!() };
        let out = name_for(dst, elem, b);
        if self.all_queues {
            b.line(&format!("{}: {} = latest_queue({})", out, elem.ir(), s.text));
        } else {
            b.line(&format!("{}: {} = latest {}", out, elem.ir(), s.text));
        }
        Ok(Val { text: out, ty: elem.as_ref().clone(), literal: false })
    }

    /// one push, through `__push` (log 38): a tick from the virtual
    /// clock unless the ring is regular; a struct pushed field by field;
    /// a task's own output sleeps to its next tick after (log 25)
    fn emit_push(&mut self, name: &str, s: &Val, v: &Val, b: &mut Body) {
        let regular = if b.vars.contains_key(name) { self.regular_locals.contains(name) } else { self.regular.contains(name) };
        if self.device(name, b) {
            // the device stores nothing (question 45): the push is the
            // platform's write
            let v = b.materialize(v);
            b.line(&format!("__out_ch({})", v.text));
            return;
        }
        if self.is_bare(name, b) {
            // no storage (question 50, fm3 log 92): the item goes to
            // each edge out of the stream, in composition order, where
            // that edge's feature is on; an edge that is off drops it
            // (question 51)
            let v = b.materialize(v);
            let gates = match &self.bare_gates {
                Some((n, g)) if n == name => g.clone(),
                _ => self.read_gates(name, b),
            };
            let edges = self.bare_edges.get(name).cloned().unwrap_or_default();
            for ((edge, _), gate) in edges.iter().zip(&gates) {
                match gate {
                    Some(on) => {
                        b.line(&format!("if {}", on));
                        b.depth += 1;
                        b.line(&format!("{}({})", edge, v.text));
                        b.depth -= 1;
                    }
                    None => b.line(&format!("{}({})", edge, v.text)),
                }
            }
            // at a rate, a step then passes (question 52): the item was
            // pushed at now, and now moves on by the item's length,
            // whether or not an edge was on. The rate is a literal, so
            // the period is worked out here
            if let Some(&hz) = self.rates.get(name) {
                self.step(hz, b);
            }
            return;
        }
        if self.is_queue(name, b) {
            // a queue holds an item only until its reader has passed
            // it (log 89): the push checks that the slot is free
            let v = b.materialize(v);
            let word = self.queue_push(s.ty.elem());
            b.line(&format!("{}({}, {})", word, s.text, v.text));
        } else if regular {
            let v = b.materialize(v);
            b.line(&format!("push({}, {})", s.text, v.text));
        } else {
            self.emit_push_only(s, v, b);
        }
        // one item, at the depth of the statement that pushes it: the
        // statement is known to have pushed something (fm3 log 103)
        if self.push_site.as_ref().is_some_and(|(n, d)| n == name && *d == b.depth) {
            self.sure_push = true;
        }
        // into a stream with a rate, from a plain function (log 93):
        // the nodes below take the item now, and then its step passes
        if let Some(hz) = self.paced(name, b) {
            self.wake(name, &s.text, true, b);
            self.trigger(name, b);
            self.step(hz, b);
        }
        if let BodyKind::Task { out, hz } = &b.kind {
            // no wiring in the store has a rate: every `__hz` is 0 and
            // the sleep would be a branch not taken (log 83)
            if out.as_deref() == Some(name) && self.any_rated_wiring {
                let hz = hz.clone();
                b.line(&format!("__sleep({})", hz));
            }
        }
    }

    fn emit_push_only(&mut self, s: &Val, v: &Val, b: &mut Body) {
        // a literal is typed first: `__push` is a template
        let v = b.materialize(v);
        b.line(&format!("__push({}, {})", s.text, v.text));
    }

    /// `x$ << a << b while (c)`: a push per item, the stream's name on
    /// the right reading as its latest item; `while` repeats the last
    /// push for as long as the condition holds of the candidate, which
    /// it reads as `_` (log 23, 39). What each item does is `push_item`'s
    fn lower_pushes(&mut self, name: &str, s: &Val, items: &[Expr], cond: Option<&Expr>, b: &mut Body) -> Result<(), Error> {
        let file = b.file.clone();
        let Ty::Stream(elem) = s.ty.clone() else { unreachable!() };
        let elem = *elem;
        let block_ty = Ty::Stream(Box::new(elem.clone()));
        // an item is lowered as its own type, so that a `<<` method may
        // take it (log 59); a list alone takes the element as the type
        // the context fixes, since its items have none of their own
        let item = |l: &mut Lowerer, e: &Expr, b: &mut Body| -> Result<Val, Error> {
            let want = if matches!(e.kind, ExprKind::List(_)) { Some(&elem) } else { None };
            l.lower_expr(e, want, b, None)
        };
        for (i, e) in items.iter().enumerate() {
            let last = i + 1 == items.len();
            // a task call in the chain: the task runs into the stream now
            if let Some((info, args, hz)) = self.task_call(e, Some(&b.vars), &file)? {
                if b.kind == BodyKind::Reset {
                    return Err(lex::error(&file, e.line, "a task call at feature scope before a pushed item: a chain's items come before its tasks"));
                }
                if cond.is_some() && last {
                    return Err(lex::error(&file, e.line, "a task call is not repeated with `while`: the task's own chain says when it stops"));
                }
                self.loose_push = true;
                self.run_task(&info, &args, hz, s, b, e.line)?;
                continue;
            }
            // a range pushed as a block: its values, straight in (log 41)
            if let ExprKind::Range { from, to, inclusive } = &e.kind {
                if cond.is_some() && last {
                    return Err(lex::error(&file, e.line, "a block is pushed once: `while` repeats an item"));
                }
                self.lower_range(from, to, *inclusive, b, RangeSink::Into(name, s), e.line)?;
                continue;
            }
            // a string literal pushed into a stream of bytes: its bytes,
            // straight from `data`, with no ring for the literal (log 57)
            if let ExprKind::Str(text) = &e.kind {
                if elem.ir() == "u8" {
                    if cond.is_some() && last {
                        return Err(lex::error(&file, e.line, "a block is pushed once: `while` repeats an item"));
                    }
                    self.push_text(name, s, text, b);
                    continue;
                }
            }
            match cond {
                Some(c) if last => {
                    b.open_loop("", "", false);
                    b.depth += 1;
                    self.push_read = Some((name.to_string(), PushRead::Latest(s.clone(), s.ty.clone())));
                    let v = item(self, e, b);
                    self.push_read = None;
                    let v = b.materialize(&v?);
                    // in the condition `_` is the candidate and the stream's
                    // name is still its latest item (log 39)
                    self.push_read = Some((name.to_string(), PushRead::Latest(s.clone(), s.ty.clone())));
                    let outer = self.candidate.replace(v.clone());
                    let cv = self.lower_expr(c, Some(&Ty::Bool), b, None);
                    self.candidate = outer;
                    self.push_read = None;
                    let cv = cv?;
                    if cv.ty != Ty::Bool {
                        return Err(lex::error(&file, c.line, "'while' takes a bool"));
                    }
                    let cv = b.materialize(&cv);
                    if v.ty == block_ty {
                        return Err(lex::error(&file, e.line, "a block is pushed once: `while` repeats an item"));
                    }
                    b.line(&format!("if {}", cv.text));
                    b.line("else");
                    b.depth += 1;
                    b.line("break");
                    b.depth -= 1;
                    self.push_item(name, s, v, e.line, b)?;
                    b.line("continue");
                    b.depth -= 1;
                }
                _ => {
                    self.push_read = Some((name.to_string(), PushRead::Latest(s.clone(), s.ty.clone())));
                    let v = item(self, e, b);
                    self.push_read = None;
                    self.push_item(name, s, v?, e.line, b)?;
                }
            }
        }
        Ok(())
    }

    /// One item into a stream, by dispatch (section 15, log 59). A value
    /// of the element type, or a block of them, is pushed as it is; else
    /// the `<<` method the stream's type and the item's pick, a literal
    /// as its own type first (`out$ << 42` writes the digits); else a
    /// literal that fits the element; else a struct as its fields with
    /// a space between, each by this rule, and an enumeration as its
    /// case's name; else the item is refused
    fn push_item(&mut self, name: &str, s: &Val, mut v: Val, line: usize, b: &mut Body) -> Result<(), Error> {
        let file = b.file.clone();
        let Ty::Stream(elem) = s.ty.clone() else { unreachable!() };
        let elem = *elem;
        if !v.literal && v.ty == elem {
            self.emit_push(name, s, &v, b);
            return Ok(());
        }
        if v.ty == Ty::Stream(Box::new(elem.clone())) {
            // `x$ << block$`: the block's unread items, one push each
            if v.ty.items().is_none() {
                return Err(lex::error(&file, line, "a stream of structs is pushed an item at a time"));
            }
            let view = self.unread_view(&v, b);
            let n = b.tmp();
            b.line(&format!("{}: i64 = len {}", n, view));
            self.push_view(name, s, &elem, &view, &n, b);
            return Ok(());
        }
        if let Some(info) = self.find_operator("<<", &s.ty, &v, &file, line)? {
            // a literal is typed for the method: its parameter's type
            // where that is concrete, else the literal's own
            if v.literal {
                let p = &info.params[1].1;
                v.ty = if is_concrete(p) { p.clone() } else if v.text.contains('.') { float_ty() } else { int_ty() };
            }
            let v = b.materialize(&v);
            self.loose_push = true;
            match self.device_fns.get(&info.ir) {
                // the device copy (log 87): `o$` is not a value there,
                // so the call takes the item alone
                Some(dev) if self.device(name, b) => b.line(&format!("{}({})", dev, v.text)),
                _ => b.line(&format!("{}({}, {})", info.ir, s.text, v.text)),
            }
            return Ok(());
        }
        if v.literal && fits_literal(&v, &elem) {
            v.ty = elem.clone();
            self.emit_push(name, s, &v, b);
            return Ok(());
        }
        match v.ty.clone() {
            Ty::Struct(sn) => {
                let Some(TypeInfo::Struct(fields)) = self.types.get(&sn).cloned() else { unreachable!() };
                for (i, (f, t, _)) in fields.iter().enumerate() {
                    if i > 0 {
                        self.push_text(name, s, " ", b);
                    }
                    let x = b.tmp();
                    b.line(&format!("{}: {} = get {}, {}", x, t.ir(), v.text, f));
                    self.push_item(name, s, Val { text: x, ty: t.clone(), literal: false }, line, b)?;
                }
                Ok(())
            }
            Ty::Enum(en) => {
                let Some(TypeInfo::Enum(cases)) = self.types.get(&en).cloned() else { unreachable!() };
                let v = b.materialize(&v);
                for (k, case) in cases.iter().enumerate() {
                    let is = b.tmp();
                    b.line(&format!("{}: u1 = cmp.eq {}, {}", is, v.text, k));
                    b.line(&format!("if {}", is));
                    b.depth += 1;
                    self.push_text(name, s, case, b);
                    b.depth -= 1;
                }
                Ok(())
            }
            _ => Err(lex::error(&file, line, format!("'{}$' holds {} but the item is {}{}", name, zero_ty(&elem), zero_ty(&v.ty), if elem == Ty::Char { ": no `<<` method takes it" } else { "" }))),
        }
    }

    /// a string's bytes into a stream of bytes, straight from `data` (log
    /// 57): as one block, or as the byte itself when it is one (log 69)
    fn push_text(&mut self, name: &str, s: &Val, text: &str, b: &mut Body) {
        let elem = s.ty.elem().cloned().unwrap_or(Ty::Char);
        if text.len() == 1 {
            let v = Val { text: text.as_bytes()[0].to_string(), ty: elem, literal: true };
            self.emit_push(name, s, &v, b);
            return;
        }
        // a literal of at least one byte, at the statement's own depth
        if !text.is_empty() && self.push_site.as_ref().is_some_and(|(n, d)| n == name && *d == b.depth) {
            self.sure_push = true;
        }
        // a literal too short for a chunked copy to pay lands in a
        // queue an item at a time from its `data`, with no view made
        // (fm3 log 109); only where `push_view` would give the block
        // to the queue's push whole
        if !text.is_empty() && text.len() < FEW_ITEMS && self.takes_block(name, b) && self.is_queue(name, b) {
            let (p, n) = self.str_data(text, b);
            b.line(&format!("push_queue_few({}, {}, {})", s.text, p, n));
            return;
        }
        let (view, n) = self.str_view(text, b);
        self.push_view(name, s, &elem, &view, &n, b);
    }

    /// does a block pushed into this stream go to the stream's push
    /// whole? Not into the device, which is given it to write; not into
    /// a rated task's own output, a stream with no storage or a paced
    /// one, which take it an item at a time
    fn takes_block(&self, name: &str, b: &Body) -> bool {
        let own = matches!(&b.kind, BodyKind::Task { out, .. } if out.as_deref() == Some(name));
        !self.device(name, b) && !own && !self.is_bare(name, b) && self.paced(name, b).is_none()
    }

    /// the `n` items of a view pushed as one block (log 69) — one by one
    /// only into a rated task's own output, whose clock steps a period
    /// per item
    fn push_view(&mut self, name: &str, s: &Val, elem: &Ty, view: &str, n: &str, b: &mut Body) {
        if self.device(name, b) {
            b.line(&format!("__out_block({})", view));
            return;
        }
        // a stream with no storage takes a block an item at a time, each
        // through its edges (question 50)
        // ... and so does a paced one, each item at its time (log 93)
        if self.takes_block(name, b) {
            let regular = if b.vars.contains_key(name) { self.regular_locals.contains(name) } else { self.regular.contains(name) };
            let word = if self.is_queue(name, b) { self.queue_push(s.ty.elem()) } else if regular { "push".to_string() } else { "__push".to_string() };
            b.line(&format!("{}({}, {})", word, s.text, view));
            return;
        }
        let k = b.tmp();
        b.open_loop("", &format!("{}: i64 = 0", k), false);
        b.depth += 1;
        let done = b.tmp();
        b.line(&format!("{}: u1 = cmp.ge {}, {}", done, k, n));
        b.line(&format!("if {}", done));
        b.depth += 1;
        b.line("break");
        b.depth -= 1;
        let x = b.tmp();
        b.line(&format!("{}: {} = load {}, {}", x, elem.ir(), view, k));
        self.emit_push(name, s, &Val { text: x, ty: elem.clone(), literal: false }, b);
        let k2 = b.tmp();
        b.line(&format!("{}: i64 = add {}, 1", k2, k));
        b.line(&format!("continue {}", k2));
        b.depth -= 1;
    }

    /// Now reaches the next slot of a stream's beat (question 52 as
    /// refined, `fm3/time.md` "a stream has a beat", fm3 log 98). A
    /// stream with a rate has slots one period apart from its phase,
    /// which is 0 s for every stream until `restart` is built, and an
    /// item pushed into it lands in the next slot at or after the
    /// pusher's now. So before a push statement's first item the clock
    /// is rounded up to a whole multiple of the period and waited for;
    /// each item's step then leaves it on the next slot. The period is
    /// `step`'s, so slot k is at k periods whichever way it is reached.
    /// A period of one step of the clock is every time there is
    fn align(&mut self, hz: i64, b: &mut Body) {
        let period = super::store::period(hz);
        if period <= 1 {
            return;
        }
        let (p, c, u, r, t) = (b.tmp(), b.tmp(), b.tmp(), b.tmp(), b.tmp());
        b.line(&format!("{}: ptr = addr __clock", p));
        b.line(&format!("{}: i64 = load {}", c, p));
        b.line(&format!("{}: i64 = add {}, {}", u, c, period - 1));
        b.line(&format!("{}: i64 = rem {}, {}", r, u, period));
        b.line(&format!("{}: i64 = sub {}, {}", t, u, r));
        b.line(&format!("__wait({})", t));
    }

    /// a step of a rate passes (question 52, fm3 log 92): the clock read,
    /// the period added, `__wait`. A declared rate is a literal, so the
    /// period is worked out here, where `__sleep(hz)` divides at run time
    fn step(&mut self, hz: i64, b: &mut Body) {
        let (p, c, t) = (b.tmp(), b.tmp(), b.tmp());
        b.line(&format!("{}: ptr = addr __clock", p));
        b.line(&format!("{}: i64 = load {}", c, p));
        b.line(&format!("{}: i64 = add {}, {}", t, c, super::store::period(hz)));
        b.line(&format!("__wait({})", t));
    }

    /// is a push into the name paced (question 39, 52, fm3 log 93)? A
    /// plain function's push into a stored feature-scope stream declared
    /// with a rate: each item is pushed, the nodes below run, and a step
    /// passes, so every consumer acts on the item at its time
    fn paced(&self, name: &str, b: &Body) -> Option<i64> {
        if b.kind != BodyKind::Fn || b.vars.contains_key(name) || self.bare.contains(name) {
            return None;
        }
        self.rates.get(name).copied()
    }

    /// the gates of the edges out of a stream with no storage, read now:
    /// each edge's feature's effective state, or nothing where the
    /// feature is static (log 71)
    fn read_gates(&mut self, name: &str, b: &mut Body) -> Vec<Option<String>> {
        let edges = self.bare_edges.get(name).cloned().unwrap_or_default();
        let mut gates = Vec::new();
        for (_, feature) in &edges {
            if self.statics.contains(feature) {
                gates.push(None);
            } else {
                let on = self.gate(feature, None, b);
                gates.push(Some(on));
            }
        }
        gates
    }

    /// after a push or an `end` from a plain function into a stream a
    /// node reads: the scheduler runs (log 25) — under the static
    /// schedule the nodes the stream reaches that its pushers do not
    /// wake, by `__run_<stream>()`, or by the node's own call where
    /// there is one and no guard (fm3 log 103)
    fn trigger(&self, name: &str, b: &mut Body) {
        if b.kind == BodyKind::Fn && self.node_inputs.contains(name) {
            if !self.static_schedule {
                b.line("__run()");
                return;
            }
            match self.rests.get(name).map(|v| v.as_slice()).unwrap_or_default() {
                [] => {}
                [k] if !self.guard => {
                    let t = b.tmp();
                    b.line(&format!("{}: u1 = __node{}()", t, k + 1));
                }
                _ => b.line(&format!("__run_{}()", name)),
            }
        }
    }

    /// Does a task give back its stream parameter's own ring and rules
    /// (fm3 log 111)? Its IR result is the parameter moved on, and the
    /// lowering makes a new version of a local stream from the one
    /// before only through `advance` and `frame` (`rebind_stream`),
    /// which move the position alone; every other way is an assignment,
    /// a call's result, or a task run over it. So: the name stands in
    /// the body only as the stream of one of the reading words, each
    /// phrase exactly its shape, is bound by nothing, and no phrase is a
    /// task's call over it. Whatever this does not recognise is a no
    fn ring_kept(&self, stmts: &[Stmt], x: &str) -> bool {
        let bound = |v: &super::syntax::VarDecl| v.name == x;
        let init = |l: &Lowerer, v: &super::syntax::VarDecl| match &v.init {
            Some(Init::Value(e)) => l.ring_kept_in(e, x),
            Some(Init::Construct(args)) => args.iter().all(|a| l.ring_kept_in(&a.value, x)),
            Some(Init::Pushes { items, cond }) => items.iter().chain(cond.iter()).all(|e| l.ring_kept_in(e, x)),
            None => true,
        };
        stmts.iter().all(|s| match s {
            Stmt::Var(v) => !bound(v) && init(self, v) && v.rate.as_ref().is_none_or(|e| self.ring_kept_in(e, x)),
            Stmt::Multi { vars, value, .. } => vars.iter().all(|p| p.name != x) && self.ring_kept_in(value, x),
            Stmt::Assign { targets, value, .. } => targets.iter().all(|t| t.name != x) && self.ring_kept_in(value, x),
            Stmt::If { cond, then, els, .. } => self.ring_kept_in(cond, x) && self.ring_kept(then, x) && els.as_ref().is_none_or(|e| self.ring_kept(e, x)),
            Stmt::Loop { vars, cond, body, yields, into, .. } => {
                let given = match into {
                    Some(super::syntax::LoopInto::Declare(ps)) => ps.iter().all(|p| p.name != x),
                    Some(super::syntax::LoopInto::Assign(ts)) => ts.iter().all(|t| t.name != x),
                    None => true,
                };
                given && yields.iter().all(|y| y != x) && vars.iter().all(|v| !bound(v) && init(self, v)) && cond.as_ref().is_none_or(|e| self.ring_kept_in(e, x)) && self.ring_kept(body, x)
            }
            Stmt::For { var, seq, body, .. } => var != x && self.ring_kept_in(seq, x) && self.ring_kept(body, x),
            Stmt::Continue { values, .. } => values.iter().all(|e| self.ring_kept_in(e, x)),
            Stmt::Break { .. } => true,
            Stmt::Check { cond, .. } => self.ring_kept_in(cond, x),
            Stmt::Push { target, items, cond, .. } => self.ring_kept_in(target, x) && items.iter().chain(cond.iter()).all(|e| self.ring_kept_in(e, x)),
            Stmt::Expr { expr, .. } => self.ring_kept_in(expr, x),
        })
    }

    fn ring_kept_in(&self, e: &Expr, x: &str) -> bool {
        match &e.kind {
            ExprKind::Int(_) | ExprKind::Float(_) | ExprKind::Str(_) | ExprKind::Bool(_) | ExprKind::Acc => true,
            ExprKind::Name(n) | ExprKind::Seq(n) => n != x,
            ExprKind::Unit(a, _) | ExprKind::Neg(a) | ExprKind::Field(a, _) => self.ring_kept_in(a, x),
            ExprKind::List(items) => items.iter().all(|a| self.ring_kept_in(a, x)),
            ExprKind::Range { from: a, to: c, .. } | ExprKind::Bin(_, a, c) | ExprKind::Index(a, c) => self.ring_kept_in(a, x) && self.ring_kept_in(c, x),
            ExprKind::IfElse(a, c, d) => self.ring_kept_in(a, x) && self.ring_kept_in(c, x) && self.ring_kept_in(d, x),
            ExprKind::Existing(_) => false,
            ExprKind::Phrase(parts) => {
                if self.task_streams(parts).iter().any(|n| n == x) {
                    return false;
                }
                let it = |p: &Part| matches!(p, Part::Value(Expr { kind: ExprKind::Seq(n), .. }) if n == x);
                let rest: &[Part] = match parts.as_slice() {
                    [Part::Word(w), s] if it(s) && matches!(w.as_str(), "count" | "ended" | "position" | "latest" | "frame") => &[],
                    [Part::Word(w), s, Part::Word(at), arg] if it(s) && ((w == "peek" && at == "at") || (w == "advance" && at == "by")) => std::slice::from_ref(arg),
                    all => all,
                };
                rest.iter().all(|p| match p {
                    Part::Word(w) => w != x,
                    Part::Args(list) => list.iter().all(|a| self.ring_kept_in(&a.value, x)),
                    Part::Value(v) => self.ring_kept_in(v, x),
                })
            }
        }
    }

    /// has the stream nodes its pushers wake, in this body?
    fn wakes_here(&self, name: &str, b: &Body) -> bool {
        b.kind == BodyKind::Fn && !b.vars.contains_key(name) && self.wakes.get(name).is_some_and(|w| !w.is_empty())
    }

    /// The nodes a stream's pushers wake, run in line (fm3 log 103):
    /// after a push statement that pushed something, or after the
    /// stream's first `end`. Each is its reader fetched and, under its
    /// feature's gate, its output fetched, its task called, the reader
    /// stored and the queue freed where this node is its one reader.
    /// Nothing is asked: something has just arrived, or the stream has
    /// just ended, so the node is due. Where the feature is off, after
    /// a push, the drop (question 51): the reader moved past what has
    /// arrived; after an `end` nothing has arrived and there is none
    fn wake(&mut self, name: &str, s: &str, pushed: bool, b: &mut Body) {
        if !self.wakes_here(name, b) {
            return;
        }
        for k in self.wakes[name].clone() {
            let node = &self.nodes[k];
            let (pname, pty) = node.info.params[0].clone();
            let (ir, feature, out) = (node.info.ir.clone(), node.feature.clone(), node.out.clone());
            // the reader is read and written in place (fm3 log 104): the
            // context's address once, the field loaded there, and each
            // write a load, a `set` and a store, which the IR dissolves
            // to the one field (log 67). The write loads again, the
            // task's call standing between
            // ... and where the task gives back its parameter's own ring
            // and rules the field is the reader's position alone, a
            // word, `set` into the stream's value, which the push or
            // the `end` before the wake has in hand (fm3 log 111)
            let field = format!("__node{}_{}", k + 1, pname);
            let placed = self.placed.contains(&k);
            let r = if placed {
                let at = self.field_get(&field, "i64", None, b);
                let r = b.tmp();
                b.line(&format!("{}: {} = set {}, pos, {}", r, pty.ir(), s, at));
                r
            } else {
                self.field_get(&field, &pty.ir(), None, b)
            };
            let gated = !self.statics.contains(&feature);
            if gated {
                let on = self.gate(&feature, None, b);
                b.line(&format!("if {}", on));
                b.depth += 1;
            }
            let mut ops = Vec::new();
            if let Some(o) = &out {
                let ty = self.fvar(o).unwrap().ty.ir();
                ops.push(self.field_get(o, &ty, None, b));
            }
            ops.push(r.clone());
            ops.push("0: i64".into());
            let r2 = b.tmp();
            b.line(&format!("{}: {} = {}({})", r2, pty.ir(), ir, ops.join(", ")));
            if placed {
                let at = b.tmp();
                b.line(&format!("{}: i64 = get {}, pos", at, r2));
                self.field_put(&field, &at, b);
            } else {
                self.field_put(&field, &r2, b);
            }
            if self.frees(name) {
                b.line(&format!("free_queue({})", r2));
            }
            if gated {
                b.depth -= 1;
                if pushed {
                    b.line("else");
                    b.depth += 1;
                    let p = self.pushed_of(&r, b);
                    if placed {
                        self.field_put(&field, &p, b);
                        if self.frees(name) {
                            let r3 = b.tmp();
                            b.line(&format!("{}: {} = set {}, pos, {}", r3, pty.ir(), r, p));
                            b.line(&format!("free_queue({})", r3));
                        }
                    } else {
                        let r3 = b.tmp();
                        b.line(&format!("{}: {} = set {}, pos, {}", r3, pty.ir(), r, p));
                        self.field_put(&field, &r3, b);
                        if self.frees(name) {
                            b.line(&format!("free_queue({})", r3));
                        }
                    }
                    b.depth -= 1;
                }
            }
        }
    }

    /// a stream variable takes its moved reader: a new version of a
    /// local, the field of a feature variable stored
    fn rebind_stream(&mut self, name: &str, s: &Val, moved: &dyn Fn(&mut Lowerer, &str, &mut Body), b: &mut Body, line: usize) -> Result<(), Error> {
        if b.vars.contains_key(name) {
            b.assignable(name, line)?;
            let ir = b.define(name, s.ty.clone());
            moved(self, &ir, b);
        } else {
            let t = b.tmp();
            moved(self, &t, b);
            self.field_put(name, &t, b);
        }
        Ok(())
    }

    /// a `for` may not move a local stream: a `loop` carries one
    fn no_moved_stream(&self, body: &[Stmt], b: &Body) -> Result<(), Error> {
        let mut moved = Vec::new();
        moved_streams(body, &mut moved, &|parts| self.task_streams(parts));
        for n in moved {
            if let Some(v) = b.vars.get(&n) {
                if matches!(v.ty, Ty::Stream(_)) {
                    return Err(lex::error(&b.file, stmt_line(&body[0]), format!("'{}$' is moved inside a `for`: move a stream inside a `loop`, which carries it", n)));
                }
            }
        }
        Ok(())
    }

    /// the stream words of section 9 applied to a stream variable —
    /// `peek x$ at (i)`, `latest x$`, `count x$`, `advance x$ by (n)`,
    /// `frame x$`, `ended x$`, `end x$`, `x$ behind (k)`, `x$ at (t)`,
    /// `x$ from (t1) to (t2)` — or None when the phrase is not one
    fn stream_word(&mut self, parts: &[Part], b: &mut Body, dst: Option<&str>, line: usize) -> Result<Option<Val>, Error> {
        let file = b.file.clone();
        let name_of = |p: &Part| -> Option<String> {
            match p {
                Part::Value(Expr { kind: ExprKind::Seq(n), .. }) => Some(n.clone()),
                _ => None,
            }
        };
        // `time of x$`: the tick of the next unread item, and the one
        // word that times a stream by asking (log 85, question 42)
        if let [Part::Word(time), Part::Word(of), x] = parts {
            if time == "time" && of == "of" {
                if let Some(n) = name_of(x).filter(|n| self.stream_var(n, b).is_some()) {
                    let sv = self.lower_expr(&Expr { kind: ExprKind::Name(n), line }, None, b, None)?;
                    let first = sv.text.clone();
                    let (k, t) = (b.tmp(), b.tmp());
                    b.line(&format!("{}: i64, {}: i64 = position({})", k, t, first));
                    let out = name_for(dst, &Ty::Num("int".into()), b);
                    b.line(&format!("{}: int = conv {}", out, t));
                    return Ok(Some(Val { text: out, ty: Ty::Num("int".into()), literal: false }));
                }
            }
        }
        let (w, sname, rest, infix) = match parts {
            [Part::Word(w), x, rest @ ..] if name_of(x).is_some_and(|n| self.stream_var(&n, b).is_some()) => (w.clone(), name_of(x).unwrap(), rest, false),
            [x, Part::Word(w), rest @ ..] if name_of(x).is_some_and(|n| self.stream_var(&n, b).is_some()) => (w.clone(), name_of(x).unwrap(), rest, true),
            _ => return Ok(None),
        };
        let one_arg = |p: &Part| -> Option<Expr> {
            match p {
                Part::Args(a) if a.len() == 1 && a[0].name.is_none() => Some(a[0].value.clone()),
                Part::Value(e) => Some(e.clone()),
                _ => None,
            }
        };
        let ty = self.stream_var(&sname, b).unwrap();
        let Ty::Stream(elem) = ty.clone() else { unreachable!() };
        let s = self.lower_expr(&Expr { kind: ExprKind::Name(sname.clone()), line }, None, b, None)?;
        // the words that weigh two items and round between them have no
        // meaning on a ring of structs (log 88); the IR refuses them by
        // name too, and this says it with the zero line
        let no_struct = |l: &Lowerer, what: &str| -> Result<(), Error> {
            if matches!(*elem, Ty::Struct(_)) {
                let _ = l;
                return Err(lex::error(&file, line, format!("{} on a stream of structs: a struct has no midpoint, so there is no value between two of them", what)));
            }
            Ok(())
        };
        let none = Val { text: String::new(), ty: Ty::None, literal: false };
        match (w.as_str(), infix, rest) {
            ("count", false, []) => Ok(Some(self.count_of(&s, b, dst))),
            ("latest", false, []) => Ok(Some(self.latest_of(&s, &ty, b, dst)?)),
            ("peek", false, [Part::Word(at), arg]) if at == "at" => {
                let Some(a) = one_arg(arg) else { return Ok(None) };
                let iv = self.lower_expr(&a, Some(&Ty::Num("int".into())), b, None)?;
                if !matches!(iv.ty, Ty::Num(_)) {
                    return Err(lex::error(&file, a.line, "'peek' takes an integer index"));
                }
                let i = self.as_i64(&iv, b);
                Ok(Some(self.peek_at(&s, &i, b, dst)))
            }
            ("advance", false, [Part::Word(by), arg]) if by == "by" => {
                let Some(a) = one_arg(arg) else { return Ok(None) };
                let nv = self.lower_expr(&a, Some(&Ty::Num("int".into())), b, None)?;
                if !matches!(nv.ty, Ty::Num(_)) {
                    return Err(lex::error(&file, a.line, "'advance' takes an integer count"));
                }
                let n = self.as_i64(&nv, b);
                let sty = ty.clone();
                let moved = |_: &mut Lowerer, out: &str, b: &mut Body| b.line(&format!("{}: {} = advance({}, {})", out, sty.ir(), s.text, n));
                self.rebind_stream(&sname, &s, &moved, b, line)?;
                Ok(Some(none))
            }
            ("frame", false, []) => {
                // everything unread, as a new stream (a copy, log 38),
                // and the reader moved past it
                let f = b.tmp();
                let k = b.tmp();
                let sty = ty.clone();
                let ft = f.clone();
                let eir = elem.ir();
                let word = if self.all_queues { "frame_queue" } else { "frame" };
                let moved = |_: &mut Lowerer, out: &str, b: &mut Body| b.line(&format!("{}: {}[], {}: i64, {}: {} = {}({})", ft, eir, k, out, sty.ir(), word, s.text));
                self.rebind_stream(&sname, &s, &moved, b, line)?;
                Ok(Some(self.copy_view(&elem, &f, b, dst)))
            }
            ("ended", false, []) => {
                let first = s.text.clone();
                let out = name_for(dst, &Ty::Bool, b);
                b.line(&format!("{}: u1 = ended({})", out, first));
                Ok(Some(Val { text: out, ty: Ty::Bool, literal: false }))
            }
            ("end", false, []) => {
                if self.input_device(&sname, Some(b)) {
                    return Err(lex::error(&file, line, INPUT_REFUSED));
                }
                // a push into a stream of this type must go on asking
                // whether it has ended (fm3 log 108)
                self.ended.push(elem.as_ref().clone());
                // the nodes the stream's pushers wake run at its first
                // `end` and not at a second, as `fin` had it (fm3 log 103)
                if self.wakes_here(&sname, b) {
                    let was = b.tmp();
                    b.line(&format!("{}: u1 = ended({})", was, s.text));
                    b.line(&format!("end({})", s.text));
                    b.line(&format!("if {}", was));
                    b.line("else");
                    b.depth += 1;
                    self.wake(&sname, &s.text, false, b);
                    b.depth -= 1;
                } else {
                    b.line(&format!("end({})", s.text));
                }
                self.trigger(&sname, b);
                Ok(Some(none))
            }
            // the index of the next unread item, and nothing else: not a
            // time word, so it never times the stream (log 85, question 42)
            ("position", false, []) => {
                let first = s.text.clone();
                let k = b.tmp();
                b.line(&format!("{}: i64 = get {}, pos", k, first));
                let out = name_for(dst, &Ty::Num("int".into()), b);
                b.line(&format!("{}: int = conv {}", out, k));
                Ok(Some(Val { text: out, ty: Ty::Num("int".into()), literal: false }))
            }
            ("behind", true, [arg]) => {
                let Some(a) = one_arg(arg) else { return Ok(None) };
                let kv = self.lower_expr(&a, Some(&Ty::Num("int".into())), b, None)?;
                if !matches!(kv.ty, Ty::Num(_)) {
                    return Err(lex::error(&file, a.line, "'behind' takes an integer count"));
                }
                let k = self.as_i64(&kv, b);
                let h = b.tmp();
                b.line(&format!("{}: {}[] = behind {}, {}", h, elem.ir(), s.text, k));
                Ok(Some(self.copy_view(&elem, &h, b, dst)))
            }
            ("at", true, [arg]) => {
                no_struct(self, "'at'")?;
                let Some(a) = one_arg(arg) else { return Ok(None) };
                let tv = self.lower_expr(&a, Some(&Ty::Num("time".into())), b, None)?;
                if tv.ty != Ty::Num("time".into()) {
                    return Err(lex::error(&file, a.line, "'x$ at (t)' takes a time, `1500 us`, `2 ms`, `1 s`"));
                }
                let out = name_for(dst, &elem, b);
                b.line(&format!("{}: {} = sample {}, {}", out, elem.ir(), s.text, tv.text));
                Ok(Some(Val { text: out, ty: *elem, literal: false }))
            }
            ("from", true, [a1, Part::Word(to), a2]) if to == "to" => {
                no_struct(self, "'from'")?;
                let (Some(e1), Some(e2)) = (one_arg(a1), one_arg(a2)) else { return Ok(None) };
                let time = Ty::Num("time".into());
                let t1 = self.lower_expr(&e1, Some(&time), b, None)?;
                let t2 = self.lower_expr(&e2, Some(&time), b, None)?;
                if t1.ty != time || t2.ty != time {
                    return Err(lex::error(&file, line, "'x$ from (t1) to (t2)' takes two times"));
                }
                let w = b.tmp();
                b.line(&format!("{}: {}[] = window {}, {}, {}", w, elem.ir(), s.text, t1.text, t2.text));
                Ok(Some(self.copy_view(&elem, &w, b, dst)))
            }
            _ => Ok(None),
        }
    }

    fn lower_expr(&mut self, e: &Expr, want: Option<&Ty>, b: &mut Body, dst: Option<&str>) -> Result<Val, Error> {
        let file = b.file.clone();
        match &e.kind {
            ExprKind::Int(v) => {
                let ty = match want {
                    Some(Ty::Num(n)) => Ty::Num(n.clone()),
                    Some(Ty::Bool) => return Err(lex::error(&file, e.line, "a number where a bool is wanted")),
                    _ => Ty::Num("int".into()),
                };
                Ok(Val { text: v.to_string(), ty, literal: true })
            }
            ExprKind::Float(s) => {
                let ty = match want {
                    Some(Ty::Num(n)) if is_integer(n) => return Err(lex::error(&file, e.line, format!("a decimal where an {} is wanted", zero_ty(&Ty::Num(n.clone()))))),
                    Some(Ty::Num(n)) => Ty::Num(n.clone()),
                    Some(Ty::Bool) => return Err(lex::error(&file, e.line, "a decimal where a bool is wanted")),
                    _ => Ty::Num("float".into()),
                };
                Ok(Val { text: s.clone(), ty, literal: true })
            }
            ExprKind::Bool(v) => Ok(Val { text: (*v as i64).to_string(), ty: Ty::Bool, literal: true }),
            ExprKind::Seq(w) => {
                // in a push chain the stream's own name is an item (log 23)
                if let Some((n, read)) = self.push_read.clone() {
                    if &n == w {
                        let PushRead::Latest(s, ty) = read;
                        return self.latest_of(&s, &ty, b, dst);
                    }
                }
                let v = self.lower_expr(&Expr { kind: ExprKind::Name(w.clone()), line: e.line }, want, b, dst)?;
                if v.ty.elem().is_none() {
                    return Err(lex::error(&file, e.line, format!("'{}$' is not a stream: '{}' is a {}", w, w, v.ty.ir())));
                }
                Ok(v)
            }
            ExprKind::Unit(inner, u) => {
                // a time literal is the IR's exact `time`; a rate belongs
                // on a stream's declaration
                let f = match u.as_str() {
                    "s" => "seconds",
                    "ms" => "millis",
                    "us" => "micros",
                    "ns" => "nanos",
                    "hz" | "khz" => return Err(lex::error(&file, e.line, "a rate belongs on a stream's declaration: `T x$ at (n hz)`")),
                    _ => return Err(lex::error(&file, e.line, format!("'{}' is not a unit of time here: s, ms, us or ns", u))),
                };
                let ty = Ty::Num("time".into());
                let n = match inner.kind {
                    ExprKind::Int(n) => n.to_string(),
                    ExprKind::Float(_) => return Err(lex::error(&file, e.line, "a time is a whole number of s, ms, us or ns")),
                    _ => {
                        // a value with a unit: an integer, widened to the
                        // library's i64 (log 33)
                        let v = self.lower_expr(inner, None, b, None)?;
                        let integer = matches!(&v.ty, Ty::Num(t) if t == "int" || t == "uint" || (t.starts_with(['i', 'u']) && t[1..].parse::<u32>().is_ok()));
                        if !integer {
                            return Err(lex::error(&file, e.line, format!("'{}' takes a whole number, given a {}", u, v.ty.ir())));
                        }
                        let v = b.materialize(&v);
                        let w = b.tmp();
                        b.line(&format!("{}: i64 = conv {}", w, v.text));
                        w
                    }
                };
                let out = name_for(dst, &ty, b);
                b.line(&format!("{}: time = {}({})", out, f, n));
                Ok(Val { text: out, ty, literal: false })
            }
            ExprKind::Acc => match &self.candidate {
                Some(v) => Ok(v.clone()),
                None => Err(lex::error(&file, e.line, "'_' marks the accumulator of a reduction, or the candidate in a chain's `while`: it goes with a stream in an operator or a call")),
            },
            ExprKind::List(items) => self.lower_list(items, want, b, dst, e.line),
            ExprKind::Range { from, to, inclusive } => self.lower_range(from, to, *inclusive, b, RangeSink::New(dst), e.line),
            ExprKind::Index(base, idx) => {
                // `x$[i]`: the i-th unread item, `peek` (log 38)
                let sv = self.lower_expr(base, None, b, None)?;
                if sv.ty.elem().is_none() {
                    return Err(lex::error(&file, e.line, format!("an index into a {}, which has no items", sv.ty.ir())));
                }
                let iv = self.lower_expr(idx, Some(&Ty::Num("int".into())), b, None)?;
                if !matches!(iv.ty, Ty::Num(_)) {
                    return Err(lex::error(&file, idx.line, "an index is an integer"));
                }
                let i = self.as_i64(&iv, b);
                Ok(self.peek_at(&sv, &i, b, dst))
            }
            ExprKind::Str(s) => {
                // a string literal: its bytes in `data`, copied into a
                // stream of bytes each time it is evaluated (log 38).
                // It is a `char$` (question 44) unless the context is a
                // stream of another byte type: the literal is bytes, and
                // its element follows the stream it goes into (log 87)
                let elem = match want.and_then(Ty::elem) {
                    Some(e) if e.ir() == "u8" => e.clone(),
                    _ => Ty::Char,
                };
                let (v, _) = self.str_view(s, b);
                Ok(self.copy_view(&elem, &v, b, dst))
            }
            ExprKind::Name(n) => match b.vars.get(n) {
                Some(v) if v.set => Ok(Val { text: v.ir.clone(), ty: v.ty.clone(), literal: false }),
                Some(_) => Err(lex::error(&file, e.line, format!("'{}' is read before it is assigned", n))),
                None if self.fvar(n).is_some() => self.read_fvar(n, b, dst, e.line),
                None => Err(lex::error(&file, e.line, format!("'{}' is not a variable here", n))),
            },
            ExprKind::Field(base, field) => {
                // `Tristate.yes`: an enumeration's case, qualified;
                // `countdown.enabled`: a feature's switch (log 28)
                if let ExprKind::Phrase(parts) = &base.kind {
                    if let [Part::Word(w)] = parts.as_slice() {
                        if field == "enabled" && self.features.contains(w) {
                            self.reach(&format!("{}.enabled", w), w, &file, e.line)?;
                            return Ok(self.read_on(w, b, dst));
                        }
                        if let Some(TypeInfo::Enum(cases)) = self.types.get(w) {
                            let Some(i) = cases.iter().position(|c| c == field) else {
                                return Err(lex::error(&file, e.line, format!("{} has no case '{}'", w, field)));
                            };
                            return Ok(Val { text: i.to_string(), ty: Ty::Enum(w.clone()), literal: true });
                        }
                    }
                }
                let v = self.lower_expr(base, None, b, None)?;
                let Ty::Struct(name) = &v.ty else {
                    return Err(lex::error(&file, e.line, format!("'.{}' on a {}, which has no fields", field, v.ty.ir())));
                };
                let Some(TypeInfo::Struct(fields)) = self.types.get(name) else { unreachable!() };
                let Some((_, fty, _)) = fields.iter().find(|(n, _, _)| n == field) else {
                    return Err(lex::error(&file, e.line, format!("{} has no field '{}'", name, field)));
                };
                let fty = fty.clone();
                let out = name_for(dst, &fty, b);
                b.line(&format!("{}: {} = get {}, {}", out, fty.ir(), v.text, field));
                Ok(Val { text: out, ty: fty, literal: false })
            }
            ExprKind::Neg(x) => {
                let v = self.lower_expr(x, want, b, None)?;
                if !matches!(v.ty, Ty::Num(_)) {
                    return Err(lex::error(&file, e.line, "'-' takes a number"));
                }
                let v = b.materialize(&v);
                let name = name_for(dst, &v.ty, b);
                b.line(&format!("{}: {} = neg {}", name, v.ty.ir(), v.text));
                Ok(Val { text: name, ty: v.ty, literal: false })
            }
            ExprKind::Bin(op, l, r) => {
                if self.candidate.is_none() && (matches!(l.kind, ExprKind::Acc) || matches!(r.kind, ExprKind::Acc)) {
                    return self.reduce_bin(op, l, r, b, dst, e.line);
                }
                let cmp = is_comparison(op);
                // a wanted sequence types the items; a wanted number, the operands
                let operand_want = if cmp {
                    None
                } else {
                    match want {
                        Some(Ty::Stream(inner)) => Some(inner.as_ref()),
                        Some(t @ Ty::Num(_)) => Some(t),
                        _ => None,
                    }
                };
                let lv = self.lower_expr(l, operand_want, b, None)?;
                // a struct on the left: the program's own operator
                if let Ty::Struct(_) = &lv.ty {
                    let mut rv = self.lower_expr(r, None, b, None)?;
                    let Some(info) = self.find_operator(op, &lv.ty, &rv, &file, e.line)? else {
                        return Err(lex::error(&file, e.line, format!("no '{}' is defined on a {} and a {}", op, zero_ty(&lv.ty), zero_ty(&rv.ty))));
                    };
                    if rv.literal {
                        rv.ty = info.params[1].1.clone();
                    }
                    let ty = info.results[0].1.clone();
                    let name = name_for(dst, &ty, b);
                    b.line(&format!("{}: {} = {}({}, {})", name, ty.ir(), info.ir, lv.text, rv.text));
                    return Ok(Val { text: name, ty, literal: false });
                }
                let lt = lv.ty.clone();
                let rv_want = if lv.literal { operand_want } else { Some(lt.elem().unwrap_or(&lt)) };
                let rv = self.lower_expr(r, rv_want, b, None)?;
                if lv.ty.elem().is_some() || rv.ty.elem().is_some() {
                    return self.seq_bin(op, lv, rv, b, dst, e.line);
                }
                self.emit_bin(op, lv, rv, want, b, dst, e.line)
            }
            ExprKind::IfElse(c, a, d) => {
                let cv = self.lower_expr(c, Some(&Ty::Bool), b, None)?;
                if cv.ty != Ty::Bool {
                    return Err(lex::error(&file, c.line, "'if' takes a bool"));
                }
                let cv = b.materialize(&cv);
                // each arm is lowered into its own block; a literal arm
                // yields the literal, which the join's parameter types
                let start = b.out.len();
                b.depth += 1;
                let mut av = self.lower_expr(a, want, b, None)?;
                let mut a_lines = b.out.split_off(start);
                let mut dv = self.lower_expr(d, if av.literal { want } else { Some(&av.ty) }, b, None)?;
                let mut d_lines = b.out.split_off(start);
                let ty = match (av.literal, dv.literal) {
                    (true, false) => dv.ty.clone(),
                    (true, true) => want.cloned().filter(|w| fits_literal(&av, w)).unwrap_or(av.ty.clone()),
                    _ => av.ty.clone(),
                };
                // arms of two concrete types meet in the wider (log 37),
                // each converted inside its own arm
                if !av.literal && !dv.literal && av.ty != dv.ty {
                    let Some(common) = wider(&av.ty, &dv.ty) else {
                        return Err(lex::error(&file, e.line, format!("the arms of 'if' give a {} and a {}: no number type computes both", zero_ty(&av.ty), zero_ty(&dv.ty))));
                    };
                    b.out.push_str(&a_lines);
                    av = self.widen_to(&av, &common, b, None);
                    a_lines = b.out.split_off(start);
                    b.out.push_str(&d_lines);
                    dv = self.widen_to(&dv, &common, b, None);
                    d_lines = b.out.split_off(start);
                }
                b.depth -= 1;
                let ty = if av.literal || dv.literal { ty } else { av.ty.clone() };
                let name = name_for(dst, &ty, b);
                b.line(&format!("{}: {} = if {}", name, ty.ir(), cv.text));
                b.out.push_str(&a_lines);
                b.depth += 1;
                b.line(&format!("yield {}", av.text));
                b.depth -= 1;
                b.line("else");
                b.out.push_str(&d_lines);
                b.depth += 1;
                b.line(&format!("yield {}", dv.text));
                b.depth -= 1;
                Ok(Val { text: name, ty, literal: false })
            }
            ExprKind::Phrase(parts) => {
                // a lone word: a variable, the feature's `enabled`, or an
                // enumeration's case
                if let [Part::Word(w)] = parts.as_slice() {
                    if b.vars.contains_key(w) || self.fvar(w).is_some() {
                        return self.lower_expr(&Expr { kind: ExprKind::Name(w.clone()), line: e.line }, want, b, dst);
                    }
                    if w == "enabled" {
                        let cur = self.cur.clone();
                        return Ok(self.read_on(&cur, b, dst));
                    }
                    if let Some(v) = self.enum_case(w) {
                        return Ok(v);
                    }
                }
                // a type applied to arguments: a struct constructed, or a
                // number converted
                if let [Part::Word(w), Part::Args(args)] = parts.as_slice() {
                    if self.types.contains_key(w) || builtin_type(w).is_some() {
                        return match self.ty(w, false, &file, e.line)? {
                            Ty::Struct(name) => self.construct(&name, args, b, dst, e.line),
                            Ty::Enum(_) => Err(lex::error(&file, e.line, format!("{} is an enumeration: name a case", w))),
                            Ty::Stream(_) | Ty::None => Err(lex::error(&file, e.line, format!("{} cannot be constructed", w))),
                            to => {
                                let [a] = args.as_slice() else {
                                    return Err(lex::error(&file, e.line, format!("a conversion is {}(x)", w)));
                                };
                                let v = self.lower_expr(&a.value, None, b, None)?;
                                if !matches!(v.ty, Ty::Num(_) | Ty::Bool | Ty::Char) || v.ty == Ty::Bool && to == Ty::Bool {
                                    return Err(lex::error(&file, e.line, format!("{}(x) converts a number, not a {}", w, zero_ty(&v.ty))));
                                }
                                if to == Ty::Bool {
                                    return Err(lex::error(&file, e.line, "a bool is a comparison, not a conversion"));
                                }
                                // a conversion the IR cannot see is a
                                // renaming: `uint8(c)` on a char is the
                                // byte it already holds, under the type
                                // asked for. A literal keeps its `conv`,
                                // since a literal's type is what picks a
                                // method at the call around it
                                if !v.literal && to.ir() == v.ty.ir() {
                                    return Ok(Val { ty: to, ..v });
                                }
                                let v = b.materialize(&v);
                                let name = name_for(dst, &to, b);
                                b.line(&format!("{}: {} = conv {}", name, to.ir(), v.text));
                                Ok(Val { text: name, ty: to, literal: false })
                            }
                        };
                    }
                }
                // `s[0]` on a stream declared without `$` (a string):
                // the parser saw a word and a one-item list
                if let [Part::Word(w), Part::Value(Expr { kind: ExprKind::List(items), line })] = parts.as_slice() {
                    let is_stream = b.vars.get(w).map(|v| v.ty.elem().is_some()).or_else(|| self.fvar(w).map(|f| f.ty.elem().is_some()));
                    if items.len() == 1 && is_stream == Some(true) {
                        let base = Expr { kind: ExprKind::Name(w.clone()), line: *line };
                        return self.lower_expr(&Expr { kind: ExprKind::Index(Box::new(base), Box::new(items[0].clone())), line: *line }, want, b, dst);
                    }
                }
                // the stream words of section 9, on a stream
                if let Some(v) = self.stream_word(parts, b, dst, e.line)? {
                    return Ok(v);
                }
                // a unit on a variable, `m ms` (log 33): the time it names,
                // computed once at the boundary
                if let [Part::Word(w), Part::Word(u)] = parts.as_slice() {
                    if (b.vars.contains_key(w) || self.fvar(w).is_some()) && matches!(u.as_str(), "s" | "ms" | "us" | "ns") {
                        let inner = Expr { kind: ExprKind::Name(w.clone()), line: e.line };
                        let unit = Expr { kind: ExprKind::Unit(Box::new(inner), u.clone()), line: e.line };
                        return self.lower_expr(&unit, want, b, dst);
                    }
                }
                if let [Part::Word(w), rest] = parts.as_slice() {
                    let named;
                    let arg = match rest {
                        Part::Value(a) => Some(a),
                        Part::Args(a) if a.len() == 1 && a[0].name.is_none() => Some(&a[0].value),
                        Part::Word(v) if b.vars.contains_key(v) || self.fvar(v).is_some() => {
                            named = Expr { kind: ExprKind::Name(v.clone()), line: e.line };
                            Some(&named)
                        }
                        _ => None,
                    };
                    // `count (e)`: how many unread items a stream has
                    if let (true, Some(a)) = (w == "count", arg) {
                        let start = b.out.len();
                        let sv = self.lower_expr(a, None, b, None)?;
                        if sv.ty.elem().is_some() {
                            return Ok(self.count_of(&sv, b, dst));
                        }
                        b.out.truncate(start);
                    }
                }
                let is_var = |w: &str| b.vars.contains_key(w) || self.fvar(w).is_some();
                let (cands, args) = find_methods(&self.funcs, parts, &is_var, &file, e.line)?;
                let cands: Vec<FnInfo> = cands.into_iter().cloned().collect();
                if cands[0].task {
                    return Err(lex::error(&file, e.line, task_refusal(&cands[0], e)));
                }
                // the method the arguments choose (section 6, log 36)
                let (info, (vals, lifted, acc, rtys, _)) = self.choose(&cands, &args, b, e.line)?;
                self.reach(&spoken(&info), &info.feature, &file, e.line)?;
                if lifted.iter().any(|&l| l) || acc.is_some() {
                    if rtys.len() > 1 || (rtys.is_empty() && acc.is_some()) {
                        return Err(lex::error(&file, e.line, format!("'{}' over a stream: the function gives one result{}", info.key, if acc.is_some() { " to fold" } else { ", or none" })));
                    }
                    if acc.is_some() && !lifted.iter().any(|&l| l) {
                        return Err(lex::error(&file, e.line, "'_' goes with a stream among the arguments"));
                    }
                    let ir = info.ir.clone();
                    let rty = rtys.first().cloned().unwrap_or(Ty::None);
                    let f = move |_: &mut Lowerer, ev: &[Val], b: &mut Body| -> Result<Val, Error> {
                        let ops: Vec<String> = ev.iter().map(|v| v.text.clone()).collect();
                        if rty == Ty::None {
                            b.line(&format!("{}({})", ir, ops.join(", ")));
                            return Ok(Val { text: String::new(), ty: Ty::None, literal: false });
                        }
                        let name = b.tmp();
                        b.line(&format!("{}: {} = {}({})", name, rty.ir(), ir, ops.join(", ")));
                        Ok(Val { text: name, ty: rty.clone(), literal: false })
                    };
                    return self.lift(vals, lifted, acc, b, dst, e.line, &f);
                }
                let ops: Vec<String> = vals.into_iter().map(|v| v.text).collect();
                let call = format!("{}({})", info.ir, ops.join(", "));
                match rtys.len() {
                    0 => {
                        b.line(&call);
                        Ok(Val { text: String::new(), ty: Ty::None, literal: false })
                    }
                    1 => {
                        let ty = rtys[0].clone();
                        let name = name_for(dst, &ty, b);
                        b.line(&format!("{}: {} = {}", name, ty.ir(), call));
                        Ok(Val { text: name, ty, literal: false })
                    }
                    _ => Err(lex::error(&file, e.line, format!("'{}' gives several results; take them with `a, b = ...`", info.key))),
                }
            }
            ExprKind::Existing(parts) => {
                let (call, rtys) = self.existing_call(parts, b, e.line)?;
                match rtys.len() {
                    0 => {
                        b.line(&call);
                        Ok(Val { text: String::new(), ty: Ty::None, literal: false })
                    }
                    1 => {
                        let ty = rtys[0].clone();
                        let name = name_for(dst, &ty, b);
                        b.line(&format!("{}: {} = {}", name, ty.ir(), call));
                        Ok(Val { text: name, ty, literal: false })
                    }
                    _ => Err(lex::error(&file, e.line, "'existing' gives several results; take them with `a, b = existing ...`")),
                }
            }
        }
    }
}

/// a moved stream's type after a loop: the local's, or the feature variable's
fn outer_ty(vars: &HashMap<String, Var>, name: &str, l: &Lowerer) -> Ty {
    match vars.get(name) {
        Some(v) => v.ty.clone(),
        None => l.fvar(name).unwrap().ty.clone(),
    }
}

/// the name a result is defined under: the destination variable's next
/// version when the value's type is the variable's, a temporary
/// otherwise (the assignment then reports the mismatch)
fn name_for(dst: Option<&str>, ty: &Ty, b: &mut Body) -> String {
    match dst {
        Some(d) if b.vars.get(d).map(|v| &v.ty) == Some(ty) => b.define(d, ty.clone()),
        _ => b.tmp(),
    }
}

/// The streams a body asks a time of (log 73, 85, zero.md section 9):
/// the names under `x$ at (t)` with `t` not a rate, `x$ from (a) to
/// (b)` and `time of x$` — `position x$` is the index alone and asks no
/// time (question 42) — and whether one of them is a parameter of the
/// function, when any stream may be passed there and the store keeps
/// every unrated stream's ticks
/// the arrival bound's pre-pass (log 79): every push statement into a
/// feature-scope stream that is not shadowed by a parameter or a local,
/// its item count when every item is countable
fn arrivals(stmts: &[Stmt], bound: &Names, bytes: &dyn Fn(&str) -> Option<bool>, out: &mut HashMap<String, Option<i64>>) {
    let mut bound = bound.clone();
    for s in stmts {
        match s {
            Stmt::Var(v) => {
                bound.insert(v.name.clone());
            }
            Stmt::Multi { vars, .. } => {
                for p in vars {
                    bound.insert(p.name.clone());
                }
            }
            Stmt::If { then, els, .. } => {
                arrivals(then, &bound, bytes, out);
                if let Some(e) = els {
                    arrivals(e, &bound, bytes, out);
                }
            }
            Stmt::Loop { vars, body, .. } => {
                let mut inner = bound.clone();
                inner.extend(vars.iter().map(|v| v.name.clone()));
                arrivals(body, &inner, bytes, out);
            }
            Stmt::For { var, body, .. } => {
                let mut inner = bound.clone();
                inner.insert(var.clone());
                arrivals(body, &inner, bytes, out);
            }
            Stmt::Push { target, items, cond, .. } => {
                if let ExprKind::Seq(n) = &target.kind {
                    if !bound.contains(n) {
                        if let Some(b) = bytes(n) {
                            arrival(n, items, cond.as_ref(), b, out);
                        }
                    }
                }
            }
            _ => {}
        }
    }
}

/// one push statement's items, into the stream's count: the largest
/// so far, or unknown once any statement's is
fn arrival(name: &str, items: &[Expr], cond: Option<&Expr>, bytes: bool, out: &mut HashMap<String, Option<i64>>) {
    let count = if cond.is_some() { None } else { items.iter().map(|e| item_count(e, bytes)).sum::<Option<i64>>() };
    let e = out.entry(name.to_string()).or_insert(Some(0));
    *e = match (*e, count) {
        (Some(a), Some(b)) => Some(a.max(b)),
        _ => None,
    };
}

/// how many items one pushed expression is, when the text says: a
/// literal one, a string its bytes into a byte stream, a list its
/// length, a range with literal bounds its count
fn item_count(e: &Expr, bytes: bool) -> Option<i64> {
    match &e.kind {
        ExprKind::Int(_) | ExprKind::Float(_) | ExprKind::Bool(_) => Some(1),
        ExprKind::Str(s) if bytes => Some(s.len() as i64),
        ExprKind::List(items) => Some(items.len() as i64),
        ExprKind::Range { from, to, inclusive } => match (&from.kind, &to.kind) {
            (ExprKind::Int(a), ExprKind::Int(z)) => Some((a - z).abs() + *inclusive as i64),
            _ => None,
        },
        _ => None,
    }
}

/// What a set of features says of the store's streams (question 50,
/// 54): the names they mention other than as the target of a push,
/// their cases' included; the edges they wire, source and target; and
/// the names they push into, an edge's target among them. `task` says
/// whether an expression in a file is a task call
/// `call` is the lowering's own question of a phrase (`find_methods`,
/// with the names the body has bound): the arguments, where the phrase
/// is a call of a function of the store. Its words are then the
/// function's name and no reading of anything (fm3 log 106): `after
/// half()` does not read `half$`
type Called<'a> = &'a dyn Fn(&[Part], &Names, &str) -> Option<Vec<Expr>>;

fn stream_uses(features: &[super::store::FeatureDoc], task: &dyn Fn(&Expr, &str) -> bool, call: Called) -> (Names, Vec<(String, String)>, Names) {
    let none = Names::new();
    let (mut named, mut pushed) = (Names::new(), Names::new());
    let mut wires: Vec<(String, String)> = Vec::new();
    for f in features {
        for d in &f.code.decls {
            match d {
                Decl::Fn(fd) => {
                    let mut bound: Names = fd.results.iter().map(|p| p.name.clone()).collect();
                    bound.extend(fd.params().map(|p| p.name.clone()));
                    mentions(&fd.body, &bound, &|e| task(e, &f.code.file), &|p, b| call(p, b, &f.code.file), &mut named, &mut pushed);
                }
                Decl::Var(v) => mentions_init(v, &none, &|p, b| call(p, b, &f.code.file), &mut named),
                Decl::Wire(e) => mentions_in(e, &none, &|p, b| call(p, b, &f.code.file), &mut named),
                Decl::Edge { target, items, cond, .. } => {
                    items.iter().skip(1).for_each(|e| mentions_in(e, &none, &|p, b| call(p, b, &f.code.file), &mut named));
                    cond.iter().for_each(|e| mentions_in(e, &none, &|p, b| call(p, b, &f.code.file), &mut named));
                    if let (ExprKind::Seq(t), Some(Expr { kind: ExprKind::Seq(s), .. })) = (&target.kind, items.first()) {
                        wires.push((s.clone(), t.clone()));
                        pushed.insert(t.clone());
                    }
                }
                Decl::Type(_) => {}
            }
        }
        for c in &f.cases {
            mentions_in(&c.call, &none, &|p, b| call(p, b, &f.md_file), &mut named);
        }
    }
    (named, wires, pushed)
}

/// every name a block mentions other than as the target of a push
/// (question 50, fm3 log 92): a stream none of the store's text names
/// so has no word applied to it and no storage. `bound` is the names the
/// function has bound itself, which are its own; `task` says whether a
/// pushed item is a task call, which fills the stream it is pushed into
/// and so takes it as a value. `pushed` takes every name that is the
/// target of a push (question 54)
fn mentions(stmts: &[Stmt], bound: &Names, task: &dyn Fn(&Expr) -> bool, call: &dyn Fn(&[Part], &Names) -> Option<Vec<Expr>>, out: &mut Names, pushed: &mut Names) {
    let mut bound = bound.clone();
    let named = |n: &String, bound: &Names, out: &mut Names| {
        if !bound.contains(n) {
            out.insert(n.clone());
        }
    };
    for s in stmts {
        match s {
            Stmt::Var(v) => {
                mentions_init(v, &bound, call, out);
                bound.insert(v.name.clone());
            }
            Stmt::Multi { vars, value, .. } => {
                mentions_in(value, &bound, call, out);
                bound.extend(vars.iter().map(|p| p.name.clone()));
            }
            Stmt::Assign { targets, value, .. } => {
                targets.iter().for_each(|t| named(&t.name, &bound, out));
                mentions_in(value, &bound, call, out);
            }
            Stmt::Expr { expr: value, .. } | Stmt::Check { cond: value, .. } => mentions_in(value, &bound, call, out),
            Stmt::If { cond, then, els, .. } => {
                mentions_in(cond, &bound, call, out);
                mentions(then, &bound, task, call, out, pushed);
                if let Some(e) = els {
                    mentions(e, &bound, task, call, out, pushed);
                }
            }
            Stmt::Loop { vars, cond, body, into, .. } => {
                let mut inner = bound.clone();
                for v in vars {
                    mentions_init(v, &inner, call, out);
                    inner.insert(v.name.clone());
                }
                cond.iter().for_each(|e| mentions_in(e, &inner, call, out));
                mentions(body, &inner, task, call, out, pushed);
                match into {
                    Some(super::syntax::LoopInto::Declare(ps)) => bound.extend(ps.iter().map(|p| p.name.clone())),
                    Some(super::syntax::LoopInto::Assign(ts)) => ts.iter().for_each(|t| named(&t.name, &bound, out)),
                    None => {}
                }
            }
            Stmt::For { var, seq, body, .. } => {
                mentions_in(seq, &bound, call, out);
                let mut inner = bound.clone();
                inner.insert(var.clone());
                mentions(body, &inner, task, call, out, pushed);
            }
            Stmt::Continue { values, .. } => values.iter().for_each(|e| mentions_in(e, &bound, call, out)),
            Stmt::Break { .. } => {}
            Stmt::Push { target, items, cond, existing, .. } => {
                items.iter().for_each(|e| mentions_in(e, &bound, call, out));
                cond.iter().for_each(|e| mentions_in(e, &bound, call, out));
                match &target.kind {
                    ExprKind::Seq(n) if *existing || items.iter().any(|e| task(e)) => named(n, &bound, out),
                    ExprKind::Seq(n) => named(n, &bound, pushed),
                    _ => mentions_in(target, &bound, call, out),
                }
            }
        }
    }
}

fn mentions_init(v: &super::syntax::VarDecl, bound: &Names, call: &dyn Fn(&[Part], &Names) -> Option<Vec<Expr>>, out: &mut Names) {
    match &v.init {
        Some(Init::Value(e)) => mentions_in(e, bound, call, out),
        Some(Init::Construct(args)) => args.iter().for_each(|a| mentions_in(&a.value, bound, call, out)),
        Some(Init::Pushes { items, cond }) => {
            items.iter().for_each(|e| mentions_in(e, bound, call, out));
            cond.iter().for_each(|e| mentions_in(e, bound, call, out));
        }
        None => {}
    }
}

fn mentions_in(e: &Expr, bound: &Names, call: &dyn Fn(&[Part], &Names) -> Option<Vec<Expr>>, out: &mut Names) {
    match &e.kind {
        ExprKind::Seq(n) | ExprKind::Name(n) => {
            if !bound.contains(n) {
                out.insert(n.clone());
            }
        }
        ExprKind::Unit(x, _) | ExprKind::Neg(x) | ExprKind::Field(x, _) => mentions_in(x, bound, call, out),
        ExprKind::List(items) => items.iter().for_each(|x| mentions_in(x, bound, call, out)),
        ExprKind::Range { from: l, to: r, .. } | ExprKind::Bin(_, l, r) | ExprKind::Index(l, r) => {
            mentions_in(l, bound, call, out);
            mentions_in(r, bound, call, out);
        }
        ExprKind::IfElse(c, t, f) => {
            mentions_in(c, bound, call, out);
            mentions_in(t, bound, call, out);
            mentions_in(f, bound, call, out);
        }
        ExprKind::Phrase(parts) | ExprKind::Existing(parts) => {
            // a call of a function of the store mentions its arguments:
            // its words are the function's name (fm3 log 106)
            if let Some(args) = call(parts, bound) {
                args.iter().for_each(|a| mentions_in(a, bound, call, out));
                return;
            }
            for p in parts {
                match p {
                    Part::Args(list) => list.iter().for_each(|a| mentions_in(&a.value, bound, call, out)),
                    Part::Value(x) => mentions_in(x, bound, call, out),
                    // a word of a phrase may be a variable read bare
                    Part::Word(w) => {
                        if !bound.contains(w) {
                            out.insert(w.clone());
                        }
                    }
                }
            }
        }
        ExprKind::Int(_) | ExprKind::Float(_) | ExprKind::Str(_) | ExprKind::Bool(_) | ExprKind::Acc => {}
    }
}

/// what the words in a store's text say about its streams (log 73,
/// 89): which are timed, which keep history, and which a function
/// reads the items of by name
struct Words {
    timed: std::collections::HashSet<String>,
    timed_all: bool,
    kept: std::collections::HashSet<String>,
    kept_all: bool,
    read: std::collections::HashSet<String>,
}

fn time_words(stmts: &[Stmt], params: &[String], w: &mut Words) {
    for s in stmts {
        match s {
            Stmt::Var(v) => time_words_init(v, params, w),
            Stmt::Multi { value, .. } | Stmt::Assign { value, .. } | Stmt::Expr { expr: value, .. } | Stmt::Check { cond: value, .. } => time_words_in(value, params, w),
            Stmt::If { cond, then, els, .. } => {
                time_words_in(cond, params, w);
                time_words(then, params, w);
                if let Some(e) = els {
                    time_words(e, params, w);
                }
            }
            Stmt::Loop { vars, cond, body, .. } => {
                for v in vars {
                    time_words_init(v, params, w);
                }
                cond.iter().for_each(|e| time_words_in(e, params, w));
                time_words(body, params, w);
            }
            Stmt::For { seq, body, .. } => {
                time_words_in(seq, params, w);
                time_words(body, params, w);
            }
            Stmt::Continue { values, .. } => values.iter().for_each(|e| time_words_in(e, params, w)),
            Stmt::Push { items, cond, .. } => {
                items.iter().for_each(|e| time_words_in(e, params, w));
                cond.iter().for_each(|e| time_words_in(e, params, w));
            }
            Stmt::Break { .. } => {}
        }
    }
}

/// a phrase ending `at (n hz)`: a task wired at a rate, as `task_call`
/// reads it (log 83)
fn is_rated_wiring(e: &Expr) -> bool {
    matches!(&e.kind, ExprKind::Phrase(parts) if matches!(parts.as_slice(), [.., Part::Word(at), Part::Args(a)] if at == "at" && a.len() == 1 && matches!(&a[0].value.kind, ExprKind::Unit(_, u) if u == "hz" || u == "khz")))
}

/// does any local declaration in the block wire a task at a rate?
fn wired_at_a_rate(stmts: &[Stmt]) -> bool {
    stmts.iter().any(|s| match s {
        Stmt::Var(v) => matches!(&v.init, Some(Init::Value(e)) if is_rated_wiring(e)),
        Stmt::If { then, els, .. } => wired_at_a_rate(then) || els.as_ref().is_some_and(|e| wired_at_a_rate(e)),
        Stmt::Loop { body, .. } | Stmt::For { body, .. } => wired_at_a_rate(body),
        _ => false,
    })
}

fn time_words_init(v: &super::syntax::VarDecl, params: &[String], w: &mut Words) {
    match &v.init {
        Some(Init::Value(e)) => time_words_in(e, params, w),
        Some(Init::Construct(args)) => args.iter().for_each(|a| time_words_in(&a.value, params, w)),
        Some(Init::Pushes { items, cond }) => {
            items.iter().for_each(|e| time_words_in(e, params, w));
            cond.iter().for_each(|e| time_words_in(e, params, w));
        }
        None => {}
    }
}

/// the words that name a stream and read nothing of what is in it, so
/// that a queue named only by these still frees its slots (question 48)
fn no_item_read(parts: &[Part]) -> bool {
    match parts {
        [Part::Word(x), Part::Value(Expr { kind: ExprKind::Seq(_), .. })] => x == "count" || x == "ended" || x == "end" || x == "position",
        [Part::Word(t), Part::Word(o), Part::Value(Expr { kind: ExprKind::Seq(_), .. })] => t == "time" && o == "of",
        _ => false,
    }
}

fn time_words_in(e: &Expr, params: &[String], w: &mut Words) {
    match &e.kind {
        // a stream named in an expression is read: `x$` is its latest
        // item, `x$[i]` an item, `for x in x$` its unread ones
        // ... unless the name is the function's own stream parameter,
        // which is not the feature's stream however it is spelled, so
        // a task over `x$` wired to `x$` still lets the queue free (fm3
        // log 113)
        ExprKind::Seq(n) => {
            if !params.contains(n) {
                w.read.insert(n.clone());
            }
        }
        ExprKind::Unit(x, _) | ExprKind::Neg(x) | ExprKind::Field(x, _) => time_words_in(x, params, w),
        ExprKind::List(items) => items.iter().for_each(|x| time_words_in(x, params, w)),
        ExprKind::Range { from, to, .. } => {
            time_words_in(from, params, w);
            time_words_in(to, params, w);
        }
        ExprKind::Bin(_, l, r) | ExprKind::Index(l, r) => {
            time_words_in(l, params, w);
            time_words_in(r, params, w);
        }
        ExprKind::IfElse(c, t, f) => {
            time_words_in(c, params, w);
            time_words_in(t, params, w);
            time_words_in(f, params, w);
        }
        ExprKind::Phrase(parts) | ExprKind::Existing(parts) => {
            let is_rate = |a: &Part| match a {
                Part::Args(list) if list.len() == 1 => matches!(&list[0].value.kind, ExprKind::Unit(_, u) if u == "hz" || u == "khz"),
                _ => false,
            };
            // a time word times the stream, and a timed stream keeps
            // its history too; a history word keeps it without timing
            // it (log 89, question 47). A word on a stream parameter
            // marks the whole store, since any stream may be passed
            // there and the bootstrap does not chase calls
            let mut mark = |n: &String, timed: bool| {
                if params.contains(n) {
                    w.kept_all = true;
                    if timed {
                        w.timed_all = true;
                    }
                }
                w.kept.insert(n.clone());
                if timed {
                    w.timed.insert(n.clone());
                }
            };
            match parts.as_slice() {
                [Part::Value(Expr { kind: ExprKind::Seq(n), .. }), Part::Word(at), a] if at == "at" && !is_rate(a) => mark(n, true),
                [Part::Value(Expr { kind: ExprKind::Seq(n), .. }), Part::Word(from), _, Part::Word(to), _] if from == "from" && to == "to" => mark(n, true),
                [Part::Word(time), Part::Word(of), Part::Value(Expr { kind: ExprKind::Seq(n), .. })] if time == "time" && of == "of" => mark(n, true),
                [Part::Word(latest), Part::Value(Expr { kind: ExprKind::Seq(n), .. })] if latest == "latest" => mark(n, false),
                [Part::Value(Expr { kind: ExprKind::Seq(n), .. }), Part::Word(behind), _] if behind == "behind" => mark(n, false),
                _ => {}
            }
            let quiet = no_item_read(parts);
            for p in parts {
                match p {
                    Part::Args(list) => list.iter().for_each(|a| time_words_in(&a.value, params, w)),
                    Part::Value(x) => {
                        if !quiet {
                            time_words_in(x, params, w)
                        }
                    }
                    Part::Word(_) => {}
                }
            }
        }
        _ => {}
    }
}

/// the streams a block moves: the names `advance x$ by (n)` and
/// `frame x$` are applied to, anywhere in it, and the stream arguments
/// of a task call (`task` says which, log 25)
fn moved_streams(stmts: &[Stmt], out: &mut Vec<String>, task: &dyn Fn(&[Part]) -> Vec<String>) {
    for s in stmts {
        match s {
            Stmt::Var(v) => match &v.init {
                Some(Init::Value(e)) => moved_in(e, out, task),
                Some(Init::Construct(args)) => args.iter().for_each(|a| moved_in(&a.value, out, task)),
                Some(Init::Pushes { items, cond, .. }) => {
                    items.iter().for_each(|e| moved_in(e, out, task));
                    cond.iter().for_each(|e| moved_in(e, out, task));
                }
                None => {}
            },
            Stmt::Multi { value, .. } | Stmt::Assign { value, .. } | Stmt::Expr { expr: value, .. } | Stmt::Check { cond: value, .. } => moved_in(value, out, task),
            Stmt::If { cond, then, els, .. } => {
                moved_in(cond, out, task);
                moved_streams(then, out, task);
                if let Some(e) = els {
                    moved_streams(e, out, task);
                }
            }
            Stmt::Loop { vars, cond, body, .. } => {
                for v in vars {
                    if let Some(Init::Value(e)) = &v.init {
                        moved_in(e, out, task);
                    }
                }
                cond.iter().for_each(|e| moved_in(e, out, task));
                moved_streams(body, out, task);
            }
            Stmt::For { seq, body, .. } => {
                moved_in(seq, out, task);
                moved_streams(body, out, task);
            }
            Stmt::Continue { values, .. } => values.iter().for_each(|e| moved_in(e, out, task)),
            Stmt::Push { items, cond, .. } => {
                items.iter().for_each(|e| moved_in(e, out, task));
                cond.iter().for_each(|e| moved_in(e, out, task));
            }
            Stmt::Break { .. } => {}
        }
    }
}

fn moved_in(e: &Expr, out: &mut Vec<String>, task: &dyn Fn(&[Part]) -> Vec<String>) {
    let parts_in = |parts: &[Part], out: &mut Vec<String>| {
        if let [Part::Word(w), Part::Value(Expr { kind: ExprKind::Seq(n), .. }), ..] = parts {
            if (w == "advance" || w == "frame") && !out.contains(n) {
                out.push(n.clone());
            }
        }
        for n in task(parts) {
            if !out.contains(&n) {
                out.push(n);
            }
        }
        for p in parts {
            match p {
                Part::Args(list) => list.iter().for_each(|a| moved_in(&a.value, out, task)),
                Part::Value(e) => moved_in(e, out, task),
                Part::Word(_) => {}
            }
        }
    };
    match &e.kind {
        ExprKind::Phrase(parts) | ExprKind::Existing(parts) => parts_in(parts, out),
        ExprKind::Bin(_, l, r) | ExprKind::Index(l, r) | ExprKind::Range { from: l, to: r, .. } => {
            moved_in(l, out, task);
            moved_in(r, out, task);
        }
        ExprKind::Neg(x) | ExprKind::Unit(x, _) | ExprKind::Field(x, _) => moved_in(x, out, task),
        ExprKind::IfElse(c, a, d) => {
            moved_in(c, out, task);
            moved_in(a, out, task);
            moved_in(d, out, task);
        }
        ExprKind::List(items) => items.iter().for_each(|e| moved_in(e, out, task)),
        _ => {}
    }
}

/// a phrase as text, for a comment in the IR
fn phrase_text(e: &Expr) -> String {
    fn expr(e: &Expr) -> String {
        match &e.kind {
            ExprKind::Int(v) => v.to_string(),
            ExprKind::Float(s) => s.clone(),
            ExprKind::Str(s) => format!("{:?}", s),
            ExprKind::Bool(b) => b.to_string(),
            ExprKind::Name(n) => n.clone(),
            ExprKind::Seq(n) => format!("{}$", n),
            ExprKind::Unit(x, u) => format!("{} {}", expr(x), u),
            ExprKind::Phrase(parts) => parts
                .iter()
                .map(|p| match p {
                    Part::Word(w) => w.clone(),
                    Part::Args(a) => format!("({})", a.iter().map(|a| expr(&a.value)).collect::<Vec<_>>().join(", ")),
                    Part::Value(v) => expr(v),
                })
                .collect::<Vec<_>>()
                .join(" "),
            _ => "...".into(),
        }
    }
    expr(e)
}

fn stmt_line(s: &Stmt) -> usize {
    match s {
        Stmt::Var(v) => v.line,
        Stmt::Multi { line, .. } | Stmt::Assign { line, .. } | Stmt::If { line, .. } | Stmt::Loop { line, .. } | Stmt::For { line, .. } => *line,
        Stmt::Continue { line, .. } | Stmt::Break { line } | Stmt::Check { line, .. } | Stmt::Push { line, .. } | Stmt::Expr { line, .. } => *line,
    }
}

/// a function's name as spoken: its words
fn spoken(info: &FnInfo) -> String {
    info.parts.iter().filter_map(|p| if let NamePart::Word(w) = p { Some(w.as_str()) } else { None }).collect::<Vec<_>>().join(" ")
}

/// a task named where a call is: it is wired, into a stream or, as a
/// sink, alone (log 25, 57)
fn task_refusal(info: &FnInfo, e: &Expr) -> String {
    if info.results.is_empty() {
        format!("'{}' is a sink: it is wired at feature scope, `{}`, and not run here", spoken(info), phrase_text(e))
    } else {
        format!("'{}' is a task: it is wired into a stream, `{} x$ = {}`", spoken(info), task_elem(info), phrase_text(e))
    }
}

/// a task's element type, in zero's spelling where it has one
fn task_elem(info: &FnInfo) -> String {
    let Some((_, r)) = info.results.first() else { return "nothing".into() };
    match r {
        Ty::Stream(e) => match e.as_ref() {
            Ty::Num(n) if n == "u8" => "uint8".into(),
            Ty::Num(n) => match n.as_str() {
                "i8" | "i16" | "i32" | "i64" => format!("int{}", &n[1..]),
                "u16" | "u32" | "u64" => format!("uint{}", &n[1..]),
                "f32" | "f64" | "f16" => format!("float{}", &n[1..]),
                _ => n.clone(),
            },
            e => e.ir(),
        },
        t => t.ir(),
    }
}

/// may a literal be assigned to a variable of this type? A number
/// literal fits any number type, except that a decimal does not fit
/// an integer
fn fits_literal(v: &Val, ty: &Ty) -> bool {
    match (&v.ty, ty) {
        (Ty::Num(_), Ty::Num(t)) => !(v.text.contains('.') && is_integer(t)),
        // a whole number literal is a character's code point
        (Ty::Num(_), Ty::Char) => !v.text.contains('.'),
        (a, b) => a == b,
    }
}

/// an integer type, abstract or concrete, by its IR name
fn is_integer(t: &str) -> bool {
    t == "int" || t == "uint" || (t.len() > 1 && t.starts_with(['i', 'u']) && t[1..].parse::<u32>().is_ok())
}
