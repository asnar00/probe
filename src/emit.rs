//! The emitter: SSA -> machine code, using ONLY the learned encoding table.
//!
//! No instruction encodings appear in this file — every 32-bit word is
//! produced by `Encoder::encode(template, operands)` against the JSON the
//! prober learned and verified. What *does* live here is strategy:
//! instruction selection (which templates realize each SSA op), the frame
//! layout, and branch/call fixups.
//!
//! Register strategy: linear-scan allocation (src/regalloc.rs) over the
//! callee-saved pool x19..x28 — values allocated there survive calls by
//! construction, so call sites need no spill logic. Values that don't fit
//! spill to stack slots and stage through scratch x9/x10/x11 per
//! instruction. Branch arguments move in two phases through x9..x16 so
//! swap-shaped jumps can't clobber themselves.
//!
//! Frame layout (sp stays put for the whole body):
//!     sp + 0            saved x29, x30
//!     sp + 16 + 8k      save area for the k-th used callee-saved register
//!     spill_base + 8i   slot for the i-th spilled value
//!
//! Types up to 32 bits live in w registers, wider ones in x registers, and
//! every value is kept *canonical* in its container: sign-extended for
//! `iN`, zero-extended for `uN`/ptr/packs (see `ssa::Repr`). Slots are
//! always moved as x registers (bit copies preserve canonical form; the
//! high half of a w value is don't-care). Operations that can carry out
//! of an N-bit type re-normalize with sbfm/ubfm, which is also what
//! packs' get/set/pack lower to.

use crate::platform::{Native, Natives, Operand, Platform};
use crate::ssa::{BinOp, BlockId, Cond, Function, Inst, Module, Repr, Type, ValueId};
use std::collections::HashMap;

// ---------------------------------------------------------------------------
// Minimal JSON reader (only what our own encodings file contains)

#[derive(Debug)]
pub(crate) enum Json {
    S(String),
    N(i64),
    B(bool),
    A(Vec<Json>),
    O(Vec<(String, Json)>),
}

impl Json {
    pub(crate) fn get(&self, key: &str) -> Option<&Json> {
        match self {
            Json::O(pairs) => pairs.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }
    pub(crate) fn s(&self) -> Option<&str> {
        match self {
            Json::S(s) => Some(s),
            _ => None,
        }
    }
    pub(crate) fn n(&self) -> Option<i64> {
        match self {
            Json::N(n) => Some(*n),
            _ => None,
        }
    }
    pub(crate) fn a(&self) -> Option<&[Json]> {
        match self {
            Json::A(v) => Some(v),
            _ => None,
        }
    }
}

pub(crate) fn parse_json_pub(src: &str) -> Result<Json, String> {
    parse_json(src)
}

fn parse_json(src: &str) -> Result<Json, String> {
    let mut p = JsonParser {
        b: src.as_bytes(),
        i: 0,
    };
    let v = p.value()?;
    p.ws();
    if p.i != p.b.len() {
        return Err("trailing content after JSON value".into());
    }
    Ok(v)
}

struct JsonParser<'a> {
    b: &'a [u8],
    i: usize,
}

impl JsonParser<'_> {
    fn ws(&mut self) {
        while self.i < self.b.len() && (self.b[self.i] as char).is_ascii_whitespace() {
            self.i += 1;
        }
    }
    fn eat(&mut self, c: u8) -> Result<(), String> {
        self.ws();
        if self.i < self.b.len() && self.b[self.i] == c {
            self.i += 1;
            Ok(())
        } else {
            Err(format!("expected '{}' at byte {}", c as char, self.i))
        }
    }
    fn peek(&mut self) -> u8 {
        self.ws();
        *self.b.get(self.i).unwrap_or(&0)
    }
    fn value(&mut self) -> Result<Json, String> {
        match self.peek() {
            b'"' => Ok(Json::S(self.string()?)),
            b'{' => {
                self.eat(b'{')?;
                let mut pairs = Vec::new();
                if self.peek() != b'}' {
                    loop {
                        let k = self.string()?;
                        self.eat(b':')?;
                        pairs.push((k, self.value()?));
                        if self.peek() != b',' {
                            break;
                        }
                        self.eat(b',')?;
                    }
                }
                self.eat(b'}')?;
                Ok(Json::O(pairs))
            }
            b'[' => {
                self.eat(b'[')?;
                let mut items = Vec::new();
                if self.peek() != b']' {
                    loop {
                        items.push(self.value()?);
                        if self.peek() != b',' {
                            break;
                        }
                        self.eat(b',')?;
                    }
                }
                self.eat(b']')?;
                Ok(Json::A(items))
            }
            b't' => {
                self.i += 4;
                Ok(Json::B(true))
            }
            b'f' => {
                self.i += 5;
                Ok(Json::B(false))
            }
            _ => {
                let start = self.i;
                if self.peek() == b'-' {
                    self.i += 1;
                }
                while self.i < self.b.len() && self.b[self.i].is_ascii_digit() {
                    self.i += 1;
                }
                std::str::from_utf8(&self.b[start..self.i])
                    .ok()
                    .and_then(|s| s.parse().ok())
                    .map(Json::N)
                    .ok_or_else(|| format!("bad JSON number at byte {}", start))
            }
        }
    }
    fn string(&mut self) -> Result<String, String> {
        self.eat(b'"')?;
        let mut s = String::new();
        while self.i < self.b.len() {
            match self.b[self.i] {
                b'"' => {
                    self.i += 1;
                    return Ok(s);
                }
                b'\\' => {
                    self.i += 1;
                    s.push(self.b[self.i] as char);
                    self.i += 1;
                }
                c => {
                    s.push(c as char);
                    self.i += 1;
                }
            }
        }
        Err("unterminated JSON string".into())
    }
}

// ---------------------------------------------------------------------------
// Encoder: learned templates -> instruction words

enum SlotSpec {
    Reg,
    Imm { lo: i64, hi: i64, step: i64 },
    Enum { choices: Vec<String> },
}

enum FieldEnc {
    Linear { bits: Vec<u32>, signed: bool },
    Table { entries: Vec<u64> },
}

struct InstSpec {
    fixed: u64,
    fields: Vec<(SlotSpec, FieldEnc)>,
}

pub struct Encoder {
    insts: HashMap<String, InstSpec>,
}

fn parse_slot_spec(s: &str) -> Result<SlotSpec, String> {
    let inner = s
        .strip_prefix('{')
        .and_then(|s| s.strip_suffix('}'))
        .ok_or_else(|| format!("bad slot spec '{}'", s))?;
    if let Some(spec) = inner.strip_prefix("i ") {
        let (range, step) = match spec.split_once('/') {
            Some((r, st)) => (r.trim(), st.trim().parse::<i64>().map_err(|e| e.to_string())?),
            None => (spec.trim(), 1),
        };
        let dots = range[1..].find("..").map(|i| i + 1).ok_or("bad imm range")?;
        let lo: i64 = range[..dots].parse().map_err(|_| "bad imm lo")?;
        let hi: i64 = range[dots + 2..].parse().map_err(|_| "bad imm hi")?;
        Ok(SlotSpec::Imm { lo, hi, step })
    } else if let Some(spec) = inner.strip_prefix("e ") {
        Ok(SlotSpec::Enum {
            choices: spec.split('|').map(|c| c.trim().to_string()).collect(),
        })
    } else {
        Ok(SlotSpec::Reg)
    }
}

impl Encoder {
    pub fn load(path: &str) -> Result<Encoder, String> {
        let src = std::fs::read_to_string(path).map_err(|e| format!("{}: {}", path, e))?;
        let root = parse_json(&src)?;
        let mut insts = HashMap::new();
        for item in root
            .get("instructions")
            .and_then(Json::a)
            .ok_or("no 'instructions' array")?
        {
            let template = item
                .get("template")
                .and_then(Json::s)
                .ok_or("instruction with no template")?
                .to_string();
            let fixed_str = item.get("fixed").and_then(Json::s).ok_or("no fixed")?;
            let fixed = u64::from_str_radix(fixed_str.trim_start_matches("0x"), 16)
                .map_err(|e| format!("bad fixed '{}': {}", fixed_str, e))?;
            let mut fields = Vec::new();
            for f in item.get("fields").and_then(Json::a).unwrap_or(&[]) {
                let slot = parse_slot_spec(f.get("slot").and_then(Json::s).ok_or("no slot")?)?;
                let enc = match f.get("kind").and_then(Json::s) {
                    Some("linear") => FieldEnc::Linear {
                        bits: f
                            .get("bits")
                            .and_then(Json::a)
                            .ok_or("no bits")?
                            .iter()
                            .map(|b| b.n().map(|n| n as u32).ok_or("bad bit"))
                            .collect::<Result<_, _>>()?,
                        signed: matches!(f.get("signed"), Some(Json::B(true))),
                    },
                    Some("table") => FieldEnc::Table {
                        entries: f
                            .get("entries")
                            .and_then(Json::a)
                            .ok_or("no entries")?
                            .iter()
                            .map(|e| {
                                e.s()
                                    .and_then(|s| {
                                        u64::from_str_radix(s.trim_start_matches("0x"), 16).ok()
                                    })
                                    .ok_or("bad table entry")
                            })
                            .collect::<Result<_, _>>()?,
                    },
                    k => return Err(format!("unknown field kind {:?}", k)),
                };
                fields.push((slot, enc));
            }
            insts.insert(template, InstSpec { fixed, fields });
        }
        Ok(Encoder { insts })
    }

    /// the templates whose fixed bits a word carries: the reverse of
    /// encoding, for the footprint of compiled code
    pub fn decode(&self, word: u32) -> Vec<&str> {
        let mut out: Vec<(&str, u32)> = Vec::new();
        for (t, spec) in &self.insts {
            let mut field_mask = 0u64;
            for (_, enc) in &spec.fields {
                match enc {
                    FieldEnc::Linear { bits, .. } => {
                        for b in bits {
                            field_mask |= 1 << b;
                        }
                    }
                    FieldEnc::Table { entries } => {
                        for e in entries {
                            field_mask |= e;
                        }
                    }
                }
            }
            let fixed_mask = !field_mask & 0xffff_ffff;
            if (word as u64) & fixed_mask == spec.fixed & fixed_mask {
                out.push((t.as_str(), fixed_mask.count_ones()));
            }
        }
        // the most specific match first (most fixed bits)
        out.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));
        out.into_iter().map(|(t, _)| t).collect()
    }

    /// every learned template's text
    pub fn templates(&self) -> Vec<&str> {
        let mut v: Vec<&str> = self.insts.keys().map(String::as_str).collect();
        v.sort();
        v
    }

    /// Encode one instruction. `values` per slot: register number, actual
    /// immediate value (the encoder scales by the learned step), or enum
    /// choice index.
    pub fn encode(&self, template: &str, values: &[i64]) -> Result<u32, String> {
        let spec = self
            .insts
            .get(template)
            .ok_or_else(|| format!("template not in encoding table: '{}'", template))?;
        if values.len() != spec.fields.len() {
            return Err(format!(
                "'{}' takes {} operands, got {}",
                template,
                spec.fields.len(),
                values.len()
            ));
        }
        let mut word = spec.fixed;
        for ((slot, enc), &v) in spec.fields.iter().zip(values) {
            let units = match slot {
                SlotSpec::Reg | SlotSpec::Enum { .. } => v,
                SlotSpec::Imm { lo, hi, step } => {
                    if v < *lo || v > *hi || v % step != 0 {
                        return Err(format!(
                            "'{}': immediate {} outside learned range {}..{} step {}",
                            template, v, lo, hi, step
                        ));
                    }
                    v / step
                }
            };
            let contribution = match enc {
                FieldEnc::Linear { bits, signed } => {
                    let n = bits.len() as u32;
                    let u = if *signed {
                        if n == 0 || units < -(1i64 << (n - 1)) || units >= (1i64 << (n - 1)) {
                            return Err(format!("'{}': {} out of field range", template, v));
                        }
                        (units as u64) & ((1u64 << n) - 1)
                    } else {
                        if units < 0 || (n < 64 && units as u64 >= 1u64 << n) {
                            return Err(format!("'{}': {} out of field range", template, v));
                        }
                        units as u64
                    };
                    let mut c = 0u64;
                    for (b, &eb) in bits.iter().enumerate() {
                        if u >> b & 1 == 1 {
                            c |= 1u64 << eb;
                        }
                    }
                    c
                }
                FieldEnc::Table { entries } => *entries
                    .get(units as usize)
                    .ok_or_else(|| format!("'{}': {} outside table", template, v))?,
            };
            word ^= contribution;
        }
        Ok(word as u32)
    }

    /// Index of a named choice in a template's enum slot (e.g. "lt" in cset).
    pub fn enum_index(&self, template: &str, name: &str) -> Result<i64, String> {
        let spec = self
            .insts
            .get(template)
            .ok_or_else(|| format!("template not in encoding table: '{}'", template))?;
        for (slot, _) in &spec.fields {
            if let SlotSpec::Enum { choices } = slot {
                return choices
                    .iter()
                    .position(|c| c == name)
                    .map(|i| i as i64)
                    .ok_or_else(|| format!("'{}' has no enum choice '{}'", template, name));
            }
        }
        Err(format!("'{}' has no enum slot", template))
    }
}

// ---------------------------------------------------------------------------
// Compilation

pub struct Compiled {
    pub code: Vec<u8>,
    pub funcs: HashMap<String, usize>, // byte offset of each function
    /// where the instructions end, and (after alignment padding) where
    /// the data items begin
    pub code_end: usize,
    #[allow(dead_code)]
    pub data_base: usize,
    /// where the data begins, on a page of its own: the JIT maps it
    /// writable, the code before it write-protected
    pub writable_from: Option<usize>,
}

// templates, named once (the strings must match the seed file exactly)
const LDR_SP: &str = "ldr {x}, [sp, #{i 0..32760 /8}]";
const STR_SP: &str = "str {x}, [sp, #{i 0..32760 /8}]";
const CSET: &str = "cset {x}, {e eq|ne|lt|le|gt|ge|lo|ls|hi|hs}";
const SBFM_W: &str = "sbfm {w}, {w}, #{i 0..31}, #{i 0..31}";
const UBFM_W: &str = "ubfm {w}, {w}, #{i 0..31}, #{i 0..31}";
const BFM_W: &str = "bfm {w}, {w}, #{i 0..31}, #{i 0..31}";
const SBFM_X: &str = "sbfm {x}, {x}, #{i 0..63}, #{i 0..63}";
const UBFM_X: &str = "ubfm {x}, {x}, #{i 0..63}, #{i 0..63}";
const BFM_X: &str = "bfm {x}, {x}, #{i 0..63}, #{i 0..63}";

/// the condition code for a comparison, by the operands' signedness
fn cond_name(c: Cond, signed: bool) -> &'static str {
    match (c, signed) {
        (Cond::Eq, _) => "eq",
        (Cond::Ne, _) => "ne",
        (Cond::Lt, true) => "lt",
        (Cond::Le, true) => "le",
        (Cond::Gt, true) => "gt",
        (Cond::Ge, true) => "ge",
        (Cond::Lt, false) => "lo",
        (Cond::Le, false) => "ls",
        (Cond::Gt, false) => "hi",
        (Cond::Ge, false) => "hs",
    }
}

/// pick the x or w form of a template by container width
fn xw(container: u32, x: &'static str, w: &'static str) -> &'static str {
    if container == 32 {
        w
    } else {
        x
    }
}

enum FixTarget {
    Block(BlockId),
    Func(String),
    /// a data item, by name: `adr` gets the distance to it
    Data(String),
}

struct Fixup {
    at: usize, // byte offset of the instruction in `code`
    template: &'static str,
    values: Vec<i64>, // offset slot holds a placeholder, patched later
    imm_slot: usize,
    target: FixTarget,
}

pub fn compile(module: &Module, enc: &Encoder) -> Result<Compiled, String> {
    compile_with(module, enc, &Platform::arm64())
}

pub fn compile_with(module: &Module, enc: &Encoder, platform: &Platform) -> Result<Compiled, String> {
    compile_image(module, enc, platform, 0)
}

/// the trap handler's name: `probe boot` installs it (see ssa.md)
pub const TRAP: &str = "__trap";
/// the interrupt handler's name: the vector table's interrupt entries
/// go here, with every register of the interrupted code kept
pub const IRQ: &str = "__irq";

/// a handler's frame: where the interrupted code's registers are kept
struct TrapFrame {
    /// the integer registers' area, from the frame base
    base: i64,
    /// an interrupt: nothing is a result, so x0 goes back too, and the
    /// float scratch registers are kept as well
    irq: bool,
    /// the integer registers kept: the caller-saved ones, and for a
    /// handler that switches tasks the callee-saved ones too — the whole
    /// file is the task's
    ints: Vec<i64>,
    /// the float registers kept (d0-d7, d16-d31; d8-d15 for a switch) and where
    fp: Vec<i64>,
    fp_base: i64,
}
/// an arm64 vector table: 16 entries of 32 instructions, 2K-aligned
const VECTOR_ENTRY: usize = 0x80;
const VECTOR_TABLE: usize = 16 * VECTOR_ENTRY;

/// Compile a module whose code will sit at byte `origin` of its image
/// (after a boot preamble): `__trap` is preceded by its vector table,
/// which must be 2K-aligned in the image, every entry a branch to it.
pub fn compile_image(module: &Module, enc: &Encoder, platform: &Platform, origin: usize) -> Result<Compiled, String> {
    let natives = platform.natives(module);
    let mut code: Vec<u8> = Vec::new();
    let mut funcs = HashMap::new();
    let mut call_fixups: Vec<Fixup> = Vec::new();

    let has_irq = module.funcs.iter().any(|f| f.name == IRQ);
    for func in &module.funcs {
        if func.name == TRAP {
            // the vector table: exceptions to __trap, interrupts (every
            // fourth entry from the second: IRQ) to __irq when there is one
            while (origin + code.len()) % VECTOR_TABLE != 0 {
                code.push(0);
            }
            for k in 0..16 {
                let target = if k % 4 == 1 && has_irq { IRQ } else { TRAP };
                let at = code.len();
                code.extend_from_slice(&enc.encode("b #{i -134217728..134217724 /4}", &[0])?.to_le_bytes());
                call_fixups.push(Fixup { at, template: "b #{i -134217728..134217724 /4}", values: vec![0], imm_slot: 0, target: FixTarget::Func(target.into()) });
                code.resize(code.len() + VECTOR_ENTRY - 4, 0);
            }
        }
        funcs.insert(func.name.clone(), code.len());
        compile_function(func, enc, &natives, &mut code, &mut call_fixups)
            .map_err(|e| format!("{}: {}", func.name, e))?;
    }

    // data after the code, on pages of its own: the program's RAM,
    // writable under the JIT as on bare metal (the code's pages stay
    // write-protected)
    let code_end = code.len();
    let (data, rw, data_offsets, rw_offsets) = crate::ssa::layout_data_parts(module);
    let writable_from = if data.is_empty() && rw.is_empty() {
        while code.len() % 8 != 0 {
            code.push(0);
        }
        None
    } else {
        while code.len() % 16384 != 0 {
            code.push(0);
        }
        Some(code.len())
    };
    let data_base = code.len();
    code.extend_from_slice(&data);
    while code.len() % 16 != 0 {
        code.push(0);
    }
    let rw_base = code.len();
    code.extend_from_slice(&rw);

    // cross-function fixups (bl) and data addresses (adr)
    for fix in call_fixups {
        let target = match &fix.target {
            FixTarget::Func(name) => *funcs.get(name.as_str()).ok_or_else(|| format!("call to undefined function {}", name))?,
            FixTarget::Data(name) => match data_offsets.get(name.as_str()) {
                Some(off) => data_base + off,
                None => rw_base + *rw_offsets.get(name.as_str()).ok_or_else(|| format!("no data named {}", name))?,
            },
            FixTarget::Block(_) => unreachable!(),
        };
        let mut values = fix.values;
        values[fix.imm_slot] = target as i64 - fix.at as i64;
        let word = enc.encode(fix.template, &values)?;
        code[fix.at..fix.at + 4].copy_from_slice(&word.to_le_bytes());
    }

    Ok(Compiled { code, funcs, code_end, data_base, writable_from })
}

struct FnEmit<'a> {
    enc: &'a Encoder,
    func: &'a Function,
    natives: &'a Natives,
    code: &'a mut Vec<u8>,
    frame: i64,
    alloc: &'a crate::regalloc::Alloc,
    /// per value: the platform's float register class (`s`/`d`), if any
    classes: Vec<Option<String>>,
    spill_base: i64,
    /// a trap or interrupt handler: the caller-saved registers of the
    /// code it interrupted are kept in the frame and it leaves by eret
    trap: Option<TrapFrame>,
    /// each `scratch` value's offset from sp
    scratch: HashMap<ValueId, i64>,
    /// the block being emitted: a jump to the next one is a fall-through
    cur: usize,
    block_offsets: Vec<Option<usize>>,
    fixups: Vec<Fixup>,
}

/// the caller-saved registers a trap handler preserves, in pairs; x30
/// is in the frame's fp/lr pair already
const TRAP_SAVED: &[i64] = &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18];
/// the float registers an interrupt handler keeps: the caller-saved
/// ones (v8-v15's low halves are callee-saved and kept by the allocator)
const IRQ_FP_SAVED: &[i64] = &[0, 1, 2, 3, 4, 5, 6, 7, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30, 31];
/// what a task switch keeps on top of those: the callee-saved registers
const SWITCH_SAVED: &[i64] = &[19, 20, 21, 22, 23, 24, 25, 26, 27, 28];
const SWITCH_FP_SAVED: &[i64] = &[8, 9, 10, 11, 12, 13, 14, 15];

impl FnEmit<'_> {
    fn emit(&mut self, template: &str, values: &[i64]) -> Result<usize, String> {
        let at = self.code.len();
        let word = self.enc.encode(template, values)?;
        self.code.extend_from_slice(&word.to_le_bytes());
        Ok(at)
    }

    fn patch(&mut self, at: usize, template: &'static str, values: &[i64]) -> Result<(), String> {
        let word = self.enc.encode(template, values)?;
        self.code[at..at + 4].copy_from_slice(&word.to_le_bytes());
        Ok(())
    }

    fn slot_off(&self, idx: usize) -> i64 {
        self.spill_base + 8 * idx as i64
    }

    /// does v live in a float register?
    fn is_f(&self, v: ValueId) -> bool {
        self.classes[v.0 as usize].is_some()
    }

    /// x <- the bits of a float register (the value's width decides the form)
    fn f_to_x(&mut self, x: i64, fr: i64, v: ValueId) -> Result<(), String> {
        let t = if self.repr(v).container() == 32 { "fmov {w}, {s}" } else { "fmov {x}, {d}" };
        self.emit(t, &[x, fr]).map(|_| ())
    }

    fn x_to_f(&mut self, fr: i64, x: i64, v: ValueId) -> Result<(), String> {
        let t = if self.repr(v).container() == 32 { "fmov {s}, {w}" } else { "fmov {d}, {x}" };
        self.emit(t, &[fr, x]).map(|_| ())
    }

    /// the float register holding v: its own, or the spill slot reloaded
    /// into `fscratch` (v16..v19: caller-saved, never allocated)
    fn src_freg(&mut self, v: ValueId, fscratch: i64) -> Result<i64, String> {
        match self.alloc.loc[v.0 as usize] {
            crate::regalloc::Loc::Reg(r) => Ok(r),
            crate::regalloc::Loc::Slot(i) => {
                let off = self.slot_off(i);
                self.emit(LDR_D_SP, &[fscratch, off])?;
                Ok(fscratch)
            }
        }
    }

    fn dst_freg(&self, v: ValueId, fscratch: i64) -> i64 {
        match self.alloc.loc[v.0 as usize] {
            crate::regalloc::Loc::Reg(r) => r,
            crate::regalloc::Loc::Slot(_) => fscratch,
        }
    }

    fn finish_f(&mut self, v: ValueId, fr: i64) -> Result<(), String> {
        if let crate::regalloc::Loc::Slot(i) = self.alloc.loc[v.0 as usize] {
            let off = self.slot_off(i);
            self.emit(STR_D_SP, &[fr, off])?;
        }
        Ok(())
    }

    /// integer register currently holding v's bits: its allocated
    /// register, the spill slot loaded into `scratch` — or, for a value
    /// living in a float register, its bits moved into `scratch`
    fn src_reg(&mut self, v: ValueId, scratch: i64) -> Result<i64, String> {
        if self.is_f(v) {
            let fr = self.src_freg(v, 16)?;
            self.f_to_x(scratch, fr, v)?;
            return Ok(scratch);
        }
        match self.alloc.loc[v.0 as usize] {
            crate::regalloc::Loc::Reg(r) => Ok(r),
            crate::regalloc::Loc::Slot(i) => {
                let off = self.slot_off(i);
                self.emit(LDR_SP, &[scratch, off])?;
                Ok(scratch)
            }
        }
    }

    /// integer register a result should be computed into
    fn dst_reg(&self, v: ValueId, scratch: i64) -> i64 {
        if self.is_f(v) {
            return scratch;
        }
        match self.alloc.loc[v.0 as usize] {
            crate::regalloc::Loc::Reg(r) => r,
            crate::regalloc::Loc::Slot(_) => scratch,
        }
    }

    /// after computing bits into dst_reg(v): spill if v lives on the
    /// stack, or move into its float register
    fn finish(&mut self, v: ValueId, reg: i64) -> Result<(), String> {
        if self.is_f(v) {
            let fr = self.dst_freg(v, 19);
            self.x_to_f(fr, reg, v)?;
            return self.finish_f(v, fr);
        }
        if let crate::regalloc::Loc::Slot(i) = self.alloc.loc[v.0 as usize] {
            let off = self.slot_off(i);
            self.emit(STR_SP, &[reg, off])?;
        }
        Ok(())
    }

    fn mov(&mut self, dst: i64, src: i64) -> Result<(), String> {
        if dst != src {
            self.emit("mov {x}, {x}", &[dst, src])?;
        }
        Ok(())
    }

    /// place v into a specific register (call args, return values, staging).
    /// Targets are x0..x17; sources are pool registers or slots — disjoint,
    /// so a sequence of these never clobbers a pending source.
    fn value_to(&mut self, target: i64, v: ValueId) -> Result<(), String> {
        if self.is_f(v) {
            let fr = self.src_freg(v, 16)?;
            return self.f_to_x(target, fr, v);
        }
        match self.alloc.loc[v.0 as usize] {
            crate::regalloc::Loc::Reg(r) => self.mov(target, r),
            crate::regalloc::Loc::Slot(i) => {
                let off = self.slot_off(i);
                self.emit(LDR_SP, &[target, off]).map(|_| ())
            }
        }
    }

    /// store a specific register into v's location
    fn value_from(&mut self, v: ValueId, source: i64) -> Result<(), String> {
        if self.is_f(v) {
            let fr = self.dst_freg(v, 16);
            self.x_to_f(fr, source, v)?;
            return self.finish_f(v, fr);
        }
        match self.alloc.loc[v.0 as usize] {
            crate::regalloc::Loc::Reg(r) => self.mov(r, source),
            crate::regalloc::Loc::Slot(i) => {
                let off = self.slot_off(i);
                self.emit(STR_SP, &[source, off]).map(|_| ())
            }
        }
    }

    fn is_next(&self, target: BlockId) -> bool {
        target.0 as usize == self.cur + 1
    }

    /// jump to a block, unless it is the next one laid out — then the
    /// code simply falls into it
    fn goto(&mut self, target: BlockId) -> Result<(), String> {
        if self.is_next(target) {
            return Ok(());
        }
        self.branch("b #{i -134217728..134217724 /4}", vec![0], 0, target)
    }

    /// branch to a block: emit with placeholder offset, fix up at function end
    fn branch(
        &mut self,
        template: &'static str,
        values: Vec<i64>,
        imm_slot: usize,
        target: BlockId,
    ) -> Result<(), String> {
        let mut placeholder = values.clone();
        placeholder[imm_slot] = 0;
        let at = self.emit(template, &placeholder)?;
        self.fixups.push(Fixup {
            at,
            template,
            values,
            imm_slot,
            target: FixTarget::Block(target),
        });
        Ok(())
    }

    /// one location-to-location move (registers or spill slots), in the
    /// integer file or (`f`) the float file
    fn loc_move(
        &mut self,
        f: bool,
        dst: crate::regalloc::Loc,
        src: crate::regalloc::Loc,
    ) -> Result<(), String> {
        use crate::regalloc::Loc;
        if f {
            return match (dst, src) {
                (Loc::Reg(d), Loc::Reg(s)) => {
                    if d != s {
                        self.emit("fmov {d}, {d}", &[d, s])?;
                    }
                    Ok(())
                }
                (Loc::Reg(d), Loc::Slot(s)) => self.emit(LDR_D_SP, &[d, self.slot_off(s)]).map(|_| ()),
                (Loc::Slot(d), Loc::Reg(s)) => self.emit(STR_D_SP, &[s, self.slot_off(d)]).map(|_| ()),
                (Loc::Slot(d), Loc::Slot(s)) => {
                    // transit through v17; v16 stays free for cycle breaking
                    self.emit(LDR_D_SP, &[17, self.slot_off(s)])?;
                    self.emit(STR_D_SP, &[17, self.slot_off(d)]).map(|_| ())
                }
            };
        }
        match (dst, src) {
            (Loc::Reg(d), Loc::Reg(s)) => self.mov(d, s),
            (Loc::Reg(d), Loc::Slot(s)) => {
                let off = self.slot_off(s);
                self.emit(LDR_SP, &[d, off]).map(|_| ())
            }
            (Loc::Slot(d), Loc::Reg(s)) => {
                let off = self.slot_off(d);
                self.emit(STR_SP, &[s, off]).map(|_| ())
            }
            (Loc::Slot(d), Loc::Slot(s)) => {
                // transit through x10; x9 stays free for cycle breaking
                self.emit(LDR_SP, &[10, self.slot_off(s)])?;
                self.emit(STR_SP, &[10, self.slot_off(d)]).map(|_| ())
            }
        }
    }

    /// move branch arguments into the target block's parameter locations:
    /// a proper parallel move — emit moves whose destination nobody still
    /// reads, break cycles (swaps, rotations) by stashing one source in x9
    fn branch_args(&mut self, target: BlockId, args: &[ValueId]) -> Result<(), String> {
        use crate::regalloc::Loc;
        let params: Vec<ValueId> = self.func.blocks[target.0 as usize].params.clone();
        // (float file?, destination, source); the two files never alias
        let mut pending: Vec<(bool, Loc, Loc)> = params
            .iter()
            .zip(args)
            .map(|(&p, &a)| (self.is_f(p), self.alloc.loc[p.0 as usize], self.alloc.loc[a.0 as usize]))
            .filter(|(_, d, s)| d != s)
            .collect();
        while !pending.is_empty() {
            if let Some(i) = (0..pending.len())
                .find(|&i| !pending.iter().any(|&(f, _, s)| f == pending[i].0 && s == pending[i].1))
            {
                let (f, d, s) = pending.swap_remove(i);
                self.loc_move(f, d, s)?;
            } else {
                // pure cycle: stash one source in the file's scratch register
                let (f, _, s) = pending[0];
                let scratch = Loc::Reg(if f { 16 } else { 9 });
                self.loc_move(f, scratch, s)?;
                for m in pending.iter_mut().filter(|m| m.0 == f && m.2 == s) {
                    m.2 = scratch;
                }
            }
        }
        Ok(())
    }

    fn repr(&self, v: ValueId) -> Repr {
        self.func.repr(self.func.ty(v))
    }

    fn movw(&mut self, dst: i64, src: i64) -> Result<(), String> {
        if dst != src {
            self.emit("mov {w}, {w}", &[dst, src])?;
        }
        Ok(())
    }

    /// a plain register copy in the container width of `r`
    fn mov_in(&mut self, r: Repr, dst: i64, src: i64) -> Result<(), String> {
        if r.container() == 32 {
            self.movw(dst, src)
        } else {
            self.mov(dst, src)
        }
    }

    /// rd = the canonical form of the bits in rs, for a value of type `r`
    /// (sign- or zero-extend the low N bits within the container)
    fn norm(&mut self, rd: i64, rs: i64, r: Repr) -> Result<(), String> {
        let n = r.bits();
        let c = r.container();
        if n == c {
            return self.mov_in(r, rd, rs);
        }
        let t = match (r.signed(), c) {
            (true, 32) => SBFM_W,
            (false, 32) => UBFM_W,
            (true, _) => SBFM_X,
            (false, _) => UBFM_X,
        };
        self.emit(t, &[rd, rs, 0, n as i64 - 1]).map(|_| ())
    }

    /// rd = the value in rs converted from canonical `from` to canonical
    /// `to` (ext, trunc, and bitcast are all this, by the verifier's rules)
    fn cast(&mut self, rd: i64, rs: i64, from: Repr, to: Repr) -> Result<(), String> {
        match (from.container(), to.container()) {
            (32, 64) => {
                // widen the container by the source's signedness, then fix
                // up the one case that changes representation (signed -> unsigned)
                if from.signed() {
                    self.emit("sxtw {x}, {w}", &[rd, rs])?;
                } else {
                    self.emit("mov {w}, {w}", &[rd, rs])?;
                }
                if !from.fits_in(to) {
                    self.norm(rd, rd, to)?;
                }
                Ok(())
            }
            (64, 32) => self.norm(rd, rs, to),
            _ => {
                if from.fits_in(to) {
                    self.mov_in(to, rd, rs)
                } else {
                    self.norm(rd, rs, to)
                }
            }
        }
    }

    /// insert the low `w` bits of rv into rd at bit `off` (rd keeps its other bits)
    fn insert(&mut self, c: u32, rd: i64, rv: i64, off: u32, w: u32) -> Result<(), String> {
        let immr = ((c - off) % c) as i64;
        self.emit(xw(c, BFM_X, BFM_W), &[rd, rv, immr, w as i64 - 1]).map(|_| ())
    }

    /// rd = field of `w` bits at `off` in rs, sign- or zero-extended per `signed`
    fn extract(&mut self, c: u32, rd: i64, rs: i64, off: u32, w: u32, signed: bool) -> Result<(), String> {
        let t = match (signed, c) {
            (true, 32) => SBFM_W,
            (false, 32) => UBFM_W,
            (true, _) => SBFM_X,
            (false, _) => UBFM_X,
        };
        self.emit(t, &[rd, rs, off as i64, (off + w) as i64 - 1]).map(|_| ())
    }

    /// the base register and immediate for an access of `size` bytes at
    /// base + off + index * step: the immediate form when the offset is
    /// in range and aligned (arm64 scales it by the access size), else
    /// the address computed into `scratch` (with `scratch2` for the
    /// scaled index)
    fn address(&mut self, base: ValueId, off: i64, index: Option<(ValueId, u32)>, size: u32, scratch: i64, scratch2: i64) -> Result<(i64, i64), String> {
        let rb = self.src_reg(base, scratch)?;
        let max = 4095 * size as i64;
        match index {
            None if off >= 0 && off <= max && off % size as i64 == 0 => Ok((rb, off)),
            None => {
                if (0..=4095).contains(&off) {
                    self.emit("add {x}, {x}, #{i 0..4095}", &[scratch, rb, off])?;
                } else if (-4095..0).contains(&off) {
                    self.emit("sub {x}, {x}, #{i 0..4095}", &[scratch, rb, -off])?;
                } else {
                    self.emit_iconst(scratch2, off)?;
                    self.emit("add {x}, {x}, {x}", &[scratch, rb, scratch2])?;
                }
                Ok((scratch, 0))
            }
            Some((i, step)) => {
                let ri = self.src_reg(i, scratch2)?;
                if step.is_power_of_two() && step > 1 {
                    self.emit("lsl {x}, {x}, #{i 0..63}", &[scratch2, ri, step.trailing_zeros() as i64])?;
                    self.emit("add {x}, {x}, {x}", &[scratch, rb, scratch2])?;
                } else if step > 1 {
                    // an array of structs: a stride that is no power of two
                    self.emit_iconst(scratch, step as i64)?;
                    self.emit("mul {x}, {x}, {x}", &[scratch2, ri, scratch])?;
                    let rb = self.src_reg(base, scratch)?;
                    self.emit("add {x}, {x}, {x}", &[scratch, rb, scratch2])?;
                } else {
                    self.emit("add {x}, {x}, {x}", &[scratch, rb, ri])?;
                }
                if off != 0 {
                    if (0..=4095).contains(&off) {
                        self.emit("add {x}, {x}, #{i 0..4095}", &[scratch, scratch, off])?;
                    } else {
                        self.emit_iconst(scratch2, off)?;
                        self.emit("add {x}, {x}, {x}", &[scratch, scratch, scratch2])?;
                    }
                }
                Ok((scratch, 0))
            }
        }
    }

    /// materialize a 64-bit constant with movz/movk, one chunk per
    /// nonzero 16 bits
    fn emit_iconst(&mut self, rd: i64, v: i64) -> Result<(), String> {
        let v = v as u64;
        let movz = [
            "movz {x}, #{i 0..65535}",
            "movz {x}, #{i 0..65535}, lsl #16",
            "movz {x}, #{i 0..65535}, lsl #32",
            "movz {x}, #{i 0..65535}, lsl #48",
        ];
        let movk = [
            "movk {x}, #{i 0..65535}",
            "movk {x}, #{i 0..65535}, lsl #16",
            "movk {x}, #{i 0..65535}, lsl #32",
            "movk {x}, #{i 0..65535}, lsl #48",
        ];
        let mut first = true;
        for i in 0..4 {
            let c = (v >> (16 * i)) as u16;
            if c == 0 {
                continue;
            }
            self.emit(if first { movz[i] } else { movk[i] }, &[rd, c as i64])?;
            first = false;
        }
        if first {
            self.emit(movz[0], &[rd, 0])?; // the constant 0
        }
        Ok(())
    }

    fn epilogue(&mut self) -> Result<(), String> {
        let fbase = 16 + 8 * self.alloc.used_regs.len() as i64;
        for (k, &fr) in self.alloc.used_by_class[1].clone().iter().enumerate() {
            self.emit(LDR_D_SP, &[fr, fbase + 8 * k as i64])?;
        }
        for (k, pair) in self.alloc.used_regs.clone().chunks(2).enumerate() {
            match pair {
                [a, b] => {
                    self.emit("ldp {x}, {x}, [sp, #{i -512..504 /8}]", &[*a, *b, 16 + 16 * k as i64])?;
                }
                [a] => {
                    self.emit(LDR_SP, &[*a, 16 + 16 * k as i64])?;
                }
                _ => unreachable!(),
            }
        }
        if let Some(t) = &self.trap {
            // a trap's result is in x0; everything else goes back as it was
            let (base, irq, ints, fp, fp_base) = (t.base, t.irq, t.ints.clone(), t.fp.clone(), t.fp_base);
            // ... and a task switch's result is the frame to go back from
            if irq && !self.func.rets.is_empty() {
                self.emit("mov sp, {x}", &[0])?;
            }
            for (k, &fr) in fp.iter().enumerate() {
                self.emit(LDR_D_SP, &[fr, fp_base + 8 * k as i64])?;
            }
            for (k, pair) in ints.chunks(2).enumerate() {
                match pair {
                    [_, b] if k == 0 && !irq => self.emit(LDR_SP, &[*b, base + 8])?,
                    [a, b] => self.emit("ldp {x}, {x}, [sp, #{i -512..504 /8}]", &[*a, *b, base + 16 * k as i64])?,
                    [a] => self.emit(LDR_SP, &[*a, base + 16 * k as i64])?,
                    _ => unreachable!(),
                };
            }
        }
        self.emit("ldp {x}, {x}, [sp, #{i -512..504 /8}]", &[29, 30, 0])?;
        self.emit("add sp, sp, #{i 0..4095}", &[self.frame])?;
        self.emit(if self.trap.is_some() { "eret" } else { "ret" }, &[])?;
        Ok(())
    }
}

/// where each `scratch` of a function goes, from `base` up, each area
/// 16-aligned: (value -> offset, the end)
pub fn scratch_layout(func: &Function, base: i64) -> (HashMap<ValueId, i64>, i64) {
    let mut at = base;
    let mut map = HashMap::new();
    for inst in func.blocks.iter().flat_map(|b| &b.insts) {
        if let Inst::Scratch { dst, bytes } = inst {
            map.insert(*dst, at);
            at += (*bytes as i64 + 15) & !15;
        }
    }
    (map, at)
}

/// pool for the allocator: callee-saved x19..x28 — values placed here
/// survive calls by construction, so call sites need no spill logic
const REG_POOL: &[i64] = &[19, 20, 21, 22, 23, 24, 25, 26, 27, 28];
/// the float file's pool: v8..v15, whose low 64 bits are callee-saved
const F_POOL: &[i64] = &[8, 9, 10, 11, 12, 13, 14, 15];
const LDR_D_SP: &str = "ldr {d}, [sp, #{i 0..32760 /8}]";
const ADR: &str = "adr {x}, #{i -1048576..1048575}";
const STR_D_SP: &str = "str {d}, [sp, #{i 0..32760 /8}]";

/// Compile one function into a standalone buffer that will live at arena
/// offset `base`; calls resolve through `resolve` (name -> arena offset of
/// the callee's entry — in the incremental arena, its trampoline).
pub fn compile_one(
    func: &Function,
    enc: &Encoder,
    natives: &Natives,
    base: i64,
    resolve: &dyn Fn(&str) -> Option<i64>,
) -> Result<Vec<u8>, String> {
    let mut code = Vec::new();
    let mut fixups = Vec::new();
    compile_function(func, enc, natives, &mut code, &mut fixups)
        .map_err(|e| format!("{}: {}", func.name, e))?;
    for fix in fixups {
        let name = match &fix.target {
            FixTarget::Func(name) => name,
            FixTarget::Data(name) => return Err(format!("data ({}) is not supported in the incremental arena yet", name)),
            FixTarget::Block(_) => unreachable!(),
        };
        let target = resolve(name).ok_or_else(|| format!("call to unknown function {}", name))?;
        let mut values = fix.values;
        values[fix.imm_slot] = target - (base + fix.at as i64);
        let word = enc.encode(fix.template, &values)?;
        code[fix.at..fix.at + 4].copy_from_slice(&word.to_le_bytes());
    }
    Ok(code)
}

fn compile_function(
    func: &Function,
    enc: &Encoder,
    natives: &Natives,
    code: &mut Vec<u8>,
    call_fixups: &mut Vec<Fixup>,
) -> Result<(), String> {
    if let Some(native) = natives.get(&func.name) {
        // this function *is* a platform instruction: x0, x1 -> x0, no frame
        native_body(enc, native, code)?;
        return Ok(());
    }
    // values of a type the platform gives a float class live in v8..v15
    let classes: Vec<Option<String>> = func.values.iter().map(|v| natives.class_of(func, v.ty).map(str::to_string)).collect();
    let class_idx: Vec<usize> = classes.iter().map(|c| c.is_some() as usize).collect();
    let alloc = crate::regalloc::allocate_classes(func, &class_idx, &[REG_POOL, F_POOL]);
    let nsaved = alloc.used_regs.len() as i64 + alloc.used_by_class[1].len() as i64;
    if func.name == IRQ && !(func.params.is_empty() && func.rets.is_empty() || func.params.len() == 1 && func.rets.len() == 1 && func.ty(func.params[0]) == Type::Ptr && func.rets[0] == Type::Ptr) {
        return Err("__irq is fn __irq() or fn __irq(sp: ptr) -> ptr".into());
    }
    let trap = (func.name == TRAP || func.name == IRQ).then(|| {
        let irq = func.name == IRQ;
        let switching = irq && !func.params.is_empty();
        let mut ints = TRAP_SAVED.to_vec();
        // an interrupt can land between any two instructions, float
        // scratch live — on a platform that has floats at all; a switch
        // hands the whole file to another task
        let mut fp: Vec<i64> = if irq && !natives.classes.is_empty() { IRQ_FP_SAVED.to_vec() } else { Vec::new() };
        if switching {
            ints.extend_from_slice(SWITCH_SAVED);
            if !natives.classes.is_empty() {
                fp.extend_from_slice(SWITCH_FP_SAVED);
            }
        }
        let base = 16 + 8 * nsaved;
        let fp_base = base + (8 * ints.len() as i64 + 15) & !15;
        TrapFrame { base, irq, ints, fp, fp_base }
    });
    let spill_base = trap.as_ref().map_or(16 + 8 * nsaved, |t| t.fp_base + 8 * t.fp.len() as i64);
    // scratch areas above the spills, each 16-aligned
    let (scratch, scratch_end) = scratch_layout(func, (spill_base + 8 * alloc.nslots as i64 + 15) & !15);
    let frame = scratch_end;
    if frame > 4095 {
        return Err(format!("function needs a {}-byte frame; 4095 is the most for now", frame));
    }
    if func.params.len() > 8 {
        return Err("more than 8 parameters not supported yet".into());
    }

    let mut e = FnEmit {
        enc,
        func,
        natives,
        code,
        frame,
        alloc: &alloc,
        classes,
        spill_base,
        trap,
        scratch,
        cur: 0,
        block_offsets: vec![None; func.blocks.len()],
        fixups: Vec::new(),
    };

    // prologue: frame, fp/lr, callee-saved save area, then parameters
    e.emit("sub sp, sp, #{i 0..4095}", &[frame])?;
    e.emit("stp {x}, {x}, [sp, #{i -512..504 /8}]", &[29, 30, 0])?;
    e.emit("mov x29, sp", &[])?;
    for (k, pair) in alloc.used_regs.chunks(2).enumerate() {
        match pair {
            [a, b] => {
                e.emit("stp {x}, {x}, [sp, #{i -512..504 /8}]", &[*a, *b, 16 + 16 * k as i64])?;
            }
            [a] => {
                e.emit(STR_SP, &[*a, 16 + 16 * k as i64])?;
            }
            _ => unreachable!(),
        }
    }
    let fbase = 16 + 8 * alloc.used_regs.len() as i64;
    for (k, &fr) in alloc.used_by_class[1].iter().enumerate() {
        e.emit(STR_D_SP, &[fr, fbase + 8 * k as i64])?;
    }
    if let Some(t) = &e.trap {
        let (base, irq, ints, fp, fp_base) = (t.base, t.irq, t.ints.clone(), t.fp.clone(), t.fp_base);
        for (k, pair) in ints.chunks(2).enumerate() {
            match pair {
                [a, b] => e.emit("stp {x}, {x}, [sp, #{i -512..504 /8}]", &[*a, *b, base + 16 * k as i64])?,
                [a] => e.emit(STR_SP, &[*a, base + 16 * k as i64])?,
                _ => unreachable!(),
            };
        }
        for (k, &fr) in fp.iter().enumerate() {
            e.emit(STR_D_SP, &[fr, fp_base + 8 * k as i64])?;
        }
        // an interrupt handler that switches tasks is given its frame:
        // where the interrupted code's registers now are
        if irq && !func.params.is_empty() {
            e.emit("add {x}, sp, #{i 0..4095}", &[0, 0])?;
        }
    }
    for (i, &p) in func.params.iter().enumerate() {
        e.value_from(p, i as i64)?;
    }

    for (bi, block) in func.blocks.iter().enumerate() {
        e.block_offsets[bi] = Some(e.code.len());
        e.cur = bi;
        for inst in &block.insts {
            compile_inst(&mut e, inst)?;
        }
    }

    // intra-function branch fixups resolve now; call fixups (function
    // targets) wait for the whole module to be laid out
    for fix in std::mem::take(&mut e.fixups) {
        match fix.target {
            FixTarget::Block(b) => {
                let target = e.block_offsets[b.0 as usize].unwrap();
                let mut values = fix.values;
                values[fix.imm_slot] = target as i64 - fix.at as i64;
                e.patch(fix.at, fix.template, &values)?;
            }
            FixTarget::Func(_) | FixTarget::Data(_) => call_fixups.push(fix),
        }
    }
    Ok(())
}

/// the instruction sequence for a platform rule: the arguments and the
/// result in the registers given, each in its own class's file
/// the registers a rule's temporaries `t0`..`t3` are: scratch ones no
/// argument or result of a rule ever lands in
const RULE_TEMPS: &[i64] = &[12, 13, 14, 15];

fn rule_seq<'a>(enc: &'a Encoder, native: &Native, rd: i64, args: &[i64]) -> Result<Vec<(&'a str, Vec<i64>)>, String> {
    let templates = enc.templates();
    let mut seq = Vec::new();
    for line in &native.rule.lines {
        if line.mnemonic == "none" {
            continue;
        }
        let (t, vals) = crate::platform::resolve(native, line, &templates)?;
        let v = vals
            .into_iter()
            .map(|(op, v)| match op {
                Operand::Arg(i) => args[i],
                Operand::Ret => rd,
                Operand::Tmp(k) => RULE_TEMPS[k],
                Operand::Lit(_) => v,
            })
            .collect();
        seq.push((t, v));
    }
    Ok(seq)
}

/// the whole body of a natively implemented function: arguments arrive
/// as bits in x0.. (the calling convention), float ones move to v16..,
/// the rule runs, a float result comes back through x0
fn native_body(enc: &Encoder, native: &Native, code: &mut Vec<u8>) -> Result<(), String> {
    let mut seq: Vec<(&str, Vec<i64>)> = Vec::new();
    let mut args = Vec::new();
    for (i, class) in native.arg_class.iter().enumerate() {
        if class.is_some() {
            let t = if native.arg_bits[i] <= 32 { "fmov {s}, {w}" } else { "fmov {d}, {x}" };
            seq.push((t, vec![16 + i as i64, i as i64]));
            args.push(16 + i as i64);
        } else {
            args.push(i as i64);
        }
    }
    let rd = if native.ret_class.is_some() { 19 } else { 0 };
    seq.extend(rule_seq(enc, native, rd, &args)?);
    if native.ret_class.is_some() {
        let t = if native.ret_bits <= 32 { "fmov {w}, {s}" } else { "fmov {x}, {d}" };
        seq.push((t, vec![0, 19]));
    }
    seq.push(("ret", vec![]));
    for (t, v) in seq {
        code.extend_from_slice(&enc.encode(t, &v)?.to_le_bytes());
    }
    Ok(())
}

fn compile_inst(e: &mut FnEmit, inst: &Inst) -> Result<(), String> {
    match inst {
        Inst::IConst { dst, imm } => {
            let rd = e.dst_reg(*dst, 9);
            let r = e.repr(*dst);
            // materialize the canonical form; a 32-bit container only
            // needs its low half
            let mut v = crate::opt::norm(r, *imm as i64) as u64;
            if r.container() == 32 {
                v &= 0xffff_ffff;
            }
            e.emit_iconst(rd, v as i64)?;
            e.finish(*dst, rd)
        }
        Inst::Bin { op, dst, lhs, rhs } => {
            let r = e.repr(*dst);
            let (n, c) = (r.bits(), r.container());
            let full = n == c; // the hardware wraps at exactly this width
            let rl = e.src_reg(*lhs, 9)?;
            let rr = e.src_reg(*rhs, 10)?;
            let rd = e.dst_reg(*dst, 9);
            match op {
                BinOp::IAdd | BinOp::ISub | BinOp::IMul | BinOp::And | BinOp::Or | BinOp::Xor => {
                    let t = match op {
                        BinOp::IAdd => xw(c, "add {x}, {x}, {x}", "add {w}, {w}, {w}"),
                        BinOp::ISub => xw(c, "sub {x}, {x}, {x}", "sub {w}, {w}, {w}"),
                        BinOp::IMul => xw(c, "mul {x}, {x}, {x}", "mul {w}, {w}, {w}"),
                        BinOp::And => xw(c, "and {x}, {x}, {x}", "and {w}, {w}, {w}"),
                        BinOp::Or => xw(c, "orr {x}, {x}, {x}", "orr {w}, {w}, {w}"),
                        _ => xw(c, "eor {x}, {x}, {x}", "eor {w}, {w}, {w}"),
                    };
                    e.emit(t, &[rd, rl, rr])?;
                    // bitwise ops of canonical values are canonical; the
                    // arithmetic ones can carry out of a narrow type
                    if !full && !matches!(op, BinOp::And | BinOp::Or | BinOp::Xor) {
                        e.norm(rd, rd, r)?;
                    }
                }
                BinOp::Div => {
                    let t = if r.signed() {
                        xw(c, "sdiv {x}, {x}, {x}", "sdiv {w}, {w}, {w}")
                    } else {
                        xw(c, "udiv {x}, {x}, {x}", "udiv {w}, {w}, {w}")
                    };
                    e.emit(t, &[rd, rl, rr])?;
                    // MIN / -1 overflows a narrow signed type; wrap it
                    if !full && r.signed() {
                        e.norm(rd, rd, r)?;
                    }
                }
                BinOp::Rem => {
                    // r = a - (a div b) * b; the quotient goes via x11
                    let div = if r.signed() {
                        xw(c, "sdiv {x}, {x}, {x}", "sdiv {w}, {w}, {w}")
                    } else {
                        xw(c, "udiv {x}, {x}, {x}", "udiv {w}, {w}, {w}")
                    };
                    e.emit(div, &[11, rl, rr])?;
                    e.emit(
                        xw(c, "msub {x}, {x}, {x}, {x}", "msub {w}, {w}, {w}, {w}"),
                        &[rd, 11, rr, rl],
                    )?;
                }
                BinOp::Shl | BinOp::Shr => {
                    let t = match (op, r.signed()) {
                        (BinOp::Shl, _) => xw(c, "lsl {x}, {x}, {x}", "lsl {w}, {w}, {w}"),
                        (_, true) => xw(c, "asr {x}, {x}, {x}", "asr {w}, {w}, {w}"),
                        (_, false) => xw(c, "lsr {x}, {x}, {x}", "lsr {w}, {w}, {w}"),
                    };
                    // the hardware shift in the container; amounts >= n
                    // are unspecified for narrow types, so nothing more is
                    // owed than re-normalizing what shl pushed out
                    e.emit(t, &[rd, rl, rr])?;
                    if !full && *op == BinOp::Shl {
                        e.norm(rd, rd, r)?;
                    }
                }
            }
            e.finish(*dst, rd)
        }
        Inst::ICmp {
            cond,
            dst,
            lhs,
            rhs,
        } => {
            let r = e.repr(*lhs);
            let rl = e.src_reg(*lhs, 9)?;
            let rr = e.src_reg(*rhs, 10)?;
            e.emit(xw(r.container(), "cmp {x}, {x}", "cmp {w}, {w}"), &[rl, rr])?;
            let rd = e.dst_reg(*dst, 9);
            let ci = e.enc.enum_index(CSET, cond_name(*cond, r.signed()))?;
            e.emit(CSET, &[rd, ci])?;
            e.finish(*dst, rd)
        }
        Inst::Cast { dst, src, .. } => {
            let from = e.repr(*src);
            let to = e.repr(*dst);
            let rs = e.src_reg(*src, 9)?;
            let rd = e.dst_reg(*dst, 10);
            e.cast(rd, rs, from, to)?;
            e.finish(*dst, rd)
        }
        Inst::Get { dst, src, field } => {
            let (off, fty) = e.func.field(e.func.ty(*src), *field).unwrap();
            let fr = e.func.repr(fty);
            let c = e.repr(*src).container();
            let rs = e.src_reg(*src, 9)?;
            let rd = e.dst_reg(*dst, 10);
            e.extract(c, rd, rs, off, fr.bits(), fr.signed())?;
            e.finish(*dst, rd)
        }
        Inst::Set {
            dst,
            src,
            field,
            val,
        } => {
            let (off, fty) = e.func.field(e.func.ty(*src), *field).unwrap();
            let w = e.func.width(fty).unwrap();
            let r = e.repr(*src);
            let rs = e.src_reg(*src, 9)?;
            let rv = e.src_reg(*val, 10)?;
            let rd = e.dst_reg(*dst, 11);
            // build in a register that is neither source
            let t = if rd == rv { 12 } else { rd };
            e.mov_in(r, t, rs)?;
            e.insert(r.container(), t, rv, off, w)?;
            e.mov_in(r, rd, t)?;
            e.finish(*dst, rd)
        }
        Inst::Pack { dst, args } => {
            let r = e.repr(*dst);
            let c = r.container();
            let ty = e.func.ty(*dst);
            let rd = e.dst_reg(*dst, 9);
            // accumulate in x12: the first field zero-extended, the rest inserted
            for (k, &a) in args.iter().enumerate() {
                let (off, fty) = e.func.field(ty, k as u32).unwrap();
                let w = e.func.width(fty).unwrap();
                let ra = e.src_reg(a, 10)?;
                if k == 0 {
                    e.extract(c, 12, ra, 0, w, false)?;
                } else {
                    e.insert(c, 12, ra, off, w)?;
                }
            }
            e.mov_in(r, rd, 12)?;
            e.finish(*dst, rd)
        }
        Inst::Unpack { dsts, src } => {
            let ty = e.func.ty(*src);
            let c = e.repr(*src).container();
            let rs = e.src_reg(*src, 9)?;
            // results may be allocated over the source; read from a copy
            e.mov(12, rs)?;
            for (k, &d) in dsts.iter().enumerate() {
                let (off, fty) = e.func.field(ty, k as u32).unwrap();
                let fr = e.func.repr(fty);
                let rd = e.dst_reg(d, 10);
                e.extract(c, rd, 12, off, fr.bits(), fr.signed())?;
                e.finish(d, rd)?;
            }
            Ok(())
        }
        Inst::Load { dst, addr, off, index } => {
            let r = e.repr(*dst);
            let (ra, imm) = e.address(*addr, *off, *index, r.bits() / 8, 9, 11)?;
            let rd = e.dst_reg(*dst, 10);
            let t = match (r.bits(), r.signed()) {
                (8, false) => "ldrb {w}, [{x}, #{i 0..4095}]",
                (8, true) => "ldrsb {w}, [{x}, #{i 0..4095}]",
                (16, false) => "ldrh {w}, [{x}, #{i 0..8190 /2}]",
                (16, true) => "ldrsh {w}, [{x}, #{i 0..8190 /2}]",
                (32, _) => "ldr {w}, [{x}, #{i 0..16380 /4}]",
                (64, _) => "ldr {x}, [{x}, #{i 0..32760 /8}]",
                (n, _) => return Err(format!("no {}-bit memory access", n)),
            };
            e.emit(t, &[rd, ra, imm])?;
            e.finish(*dst, rd)
        }
        Inst::Store { val, addr, off, index } => {
            let r = e.repr(*val);
            let rv = e.src_reg(*val, 10)?;
            let (ra, imm) = e.address(*addr, *off, *index, r.bits() / 8, 9, 11)?;
            let t = match r.bits() {
                8 => "strb {w}, [{x}, #{i 0..4095}]",
                16 => "strh {w}, [{x}, #{i 0..8190 /2}]",
                32 => "str {w}, [{x}, #{i 0..16380 /4}]",
                64 => "str {x}, [{x}, #{i 0..32760 /8}]",
                n => return Err(format!("no {}-bit memory access", n)),
            };
            e.emit(t, &[rv, ra, imm]).map(|_| ())
        }
        Inst::Addr { dst, name } => {
            let rd = e.dst_reg(*dst, 9);
            let at = e.emit(ADR, &[rd, 0])?;
            e.fixups.push(Fixup { at, template: ADR, values: vec![rd, 0], imm_slot: 1, target: FixTarget::Data(name.clone()) });
            e.finish(*dst, rd)
        }
        Inst::Scratch { dst, .. } => {
            let off = e.scratch[dst];
            let rd = e.dst_reg(*dst, 9);
            e.emit("add {x}, sp, #{i 0..4095}", &[rd, off])?;
            e.finish(*dst, rd)
        }
        Inst::Check { cond } => {
            // holds: hop over the breakpoint
            let rc = e.src_reg(*cond, 9)?;
            e.emit("cbnz {w}, #{i -1048576..1048572 /4}", &[rc, 8])?;
            e.emit("brk #{i 0..65535}", &[0]).map(|_| ())
        }
        Inst::Platform { dst, name } => {
            let v = *e.natives.consts.get(name).ok_or_else(|| format!("the platform has no constant '{}'", name))?;
            let rd = e.dst_reg(*dst, 9);
            e.emit_iconst(rd, v)?;
            e.finish(*dst, rd)
        }
        Inst::PtrAdd { dst, base, off } => {
            let rb = e.src_reg(*base, 9)?;
            let ro = e.src_reg(*off, 10)?;
            let rd = e.dst_reg(*dst, 9);
            e.emit("add {x}, {x}, {x}", &[rd, rb, ro])?;
            e.finish(*dst, rd)
        }
        Inst::Call { dsts, callee, args } if e.natives.get(callee).is_some_and(|n| n.inline) => {
            // the platform has this one: the rule's sequence instead of
            // the call, each operand in its own file
            let natives: &Natives = e.natives;
            let native = natives.get(callee).unwrap();
            let dst = dsts.first().copied();
            if dst.is_none() && native.ret_bits != 0 {
                return Ok(()); // result unused: nothing to compute
            }
            let mut regs = Vec::new();
            for (j, &a) in args.iter().enumerate() {
                regs.push(if native.arg_class[j].is_some() { e.src_freg(a, 16 + j as i64)? } else { e.src_reg(a, 9 + j as i64)? });
            }
            let ret_f = native.ret_class.is_some();
            let rd = match dst {
                Some(d) if ret_f => e.dst_freg(d, 19),
                Some(d) => e.dst_reg(d, 9),
                None => 9,
            };
            for (t, v) in rule_seq(e.enc, native, rd, &regs)? {
                e.emit(t, &v)?;
            }
            match dst {
                Some(d) if ret_f => e.finish_f(d, rd),
                Some(d) => e.finish(d, rd),
                None => Ok(()),
            }
        }
        Inst::FnAddr { dst, name } => {
            // the function's entry, pc-relative like a data item's
            let rd = e.dst_reg(*dst, 9);
            let at = e.emit(ADR, &[rd, 0])?;
            e.fixups.push(Fixup { at, template: ADR, values: vec![rd, 0], imm_slot: 1, target: FixTarget::Func(name.clone()) });
            e.finish(*dst, rd)
        }
        Inst::CallInd { dsts, callee, args } => {
            if args.len() > 8 {
                return Err("more than 8 call arguments not supported yet".into());
            }
            for (j, &a) in args.iter().enumerate() {
                e.value_to(j as i64, a)?;
            }
            // x17: neither an argument register nor callee-saved
            let rc = e.src_reg(*callee, 17)?;
            e.emit("blr {x}", &[rc])?;
            for (j, &d) in dsts.iter().enumerate() {
                e.value_from(d, j as i64)?;
            }
            Ok(())
        }
        Inst::Call { dsts, callee, args } => {
            if args.len() > 8 {
                return Err("more than 8 call arguments not supported yet".into());
            }
            for (j, &a) in args.iter().enumerate() {
                e.value_to(j as i64, a)?;
            }
            let at = e.emit("bl #{i -134217728..134217724 /4}", &[0])?;
            e.fixups.push(Fixup {
                at,
                template: "bl #{i -134217728..134217724 /4}",
                values: vec![0],
                imm_slot: 0,
                target: FixTarget::Func(callee.clone()),
            });
            for (j, &d) in dsts.iter().enumerate() {
                e.value_from(d, j as i64)?;
            }
            Ok(())
        }
        Inst::Jmp { target, args } => {
            e.branch_args(*target, args)?;
            e.goto(*target)
        }
        Inst::Br {
            cond,
            then_target,
            then_args,
            else_target,
            else_args,
        } => {
            let rc = e.src_reg(*cond, 9)?;
            if e.is_next(*then_target) && then_args.is_empty() {
                // the then block follows: branch over the else side when
                // the condition holds, and fall into it
                let cbnz_at = e.emit("cbnz {w}, #{i -1048576..1048572 /4}", &[rc, 0])?;
                e.branch_args(*else_target, else_args)?;
                e.goto(*else_target)?;
                let then_here = e.code.len() as i64 - cbnz_at as i64;
                return e.patch(cbnz_at, "cbnz {w}, #{i -1048576..1048572 /4}", &[rc, then_here]);
            }
            // cbz -> (else path, emitted after the then path); patched below
            let cbz_at = e.emit("cbz {w}, #{i -1048576..1048572 /4}", &[rc, 0])?;
            e.branch_args(*then_target, then_args)?;
            e.branch("b #{i -134217728..134217724 /4}", vec![0], 0, *then_target)?;
            let else_here = e.code.len() as i64 - cbz_at as i64;
            e.patch(
                cbz_at,
                "cbz {w}, #{i -1048576..1048572 /4}",
                &[rc, else_here],
            )?;
            e.branch_args(*else_target, else_args)?;
            e.goto(*else_target)
        }
        Inst::Ret { vals } => {
            if vals.len() > 8 {
                return Err("more than 8 return values not supported yet".into());
            }
            for (j, &v) in vals.iter().enumerate() {
                e.value_to(j as i64, v)?;
            }
            e.epilogue()
        }
    }
}

// ---------------------------------------------------------------------------
// JIT execution (macOS arm64)

#[cfg(target_os = "macos")]
pub mod jit {
    use super::Compiled;

    unsafe extern "C" {
        fn mmap(
            addr: *mut u8,
            len: usize,
            prot: i32,
            flags: i32,
            fd: i32,
            offset: i64,
        ) -> *mut u8;
        fn pthread_jit_write_protect_np(enabled: i32);
        fn sys_icache_invalidate(start: *mut u8, len: usize);
    }

    const PROT_READ: i32 = 1;
    const PROT_WRITE: i32 = 2;
    const PROT_EXEC: i32 = 4;
    const MAP_PRIVATE: i32 = 0x0002;
    const MAP_ANON: i32 = 0x1000;
    const MAP_JIT: i32 = 0x0800;
    const MAP_FIXED: i32 = 0x0010;

    pub struct JitCode {
        base: *mut u8,
        #[allow(dead_code)]
        len: usize,
        funcs: std::collections::HashMap<String, usize>,
    }

    impl JitCode {
        pub fn new(compiled: &Compiled) -> Result<JitCode, String> {
            // the code on JIT pages, write-protected once written; the
            // data, if any, on plain writable pages right after — the
            // code reaches it PC-relative, so it must be exactly where
            // the image has it
            let split = compiled.writable_from.unwrap_or(compiled.code.len());
            let len = split.next_multiple_of(16384);
            let rw_len = compiled.writable_from.map_or(0, |w| (compiled.code.len() - w).next_multiple_of(16384));
            unsafe {
                // the whole range reserved at once (so the data's pages
                // are certainly free), the code part JIT
                let base = mmap(
                    std::ptr::null_mut(),
                    len + rw_len,
                    PROT_READ | PROT_WRITE | PROT_EXEC,
                    MAP_PRIVATE | MAP_ANON | MAP_JIT,
                    -1,
                    0,
                );
                if base as isize == -1 {
                    return Err("mmap(MAP_JIT) failed".into());
                }
                pthread_jit_write_protect_np(0);
                std::ptr::copy_nonoverlapping(compiled.code.as_ptr(), base, split);
                pthread_jit_write_protect_np(1);
                sys_icache_invalidate(base, len);
                let mut total = len;
                if let Some(w) = compiled.writable_from {
                    // the tail replaced by plain writable pages (ours to replace)
                    let want = base.add(w);
                    let p = mmap(want, rw_len, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANON | MAP_FIXED, -1, 0);
                    if p != want {
                        return Err("could not place the data's pages after the code".into());
                    }
                    std::ptr::copy_nonoverlapping(compiled.code.as_ptr().add(w), p, compiled.code.len() - w);
                    total += rw_len;
                }
                Ok(JitCode {
                    base,
                    len: total,
                    funcs: compiled.funcs.clone(),
                })
            }
        }

        /// Call a compiled function with up to 6 integer arguments.
        pub fn call(&self, name: &str, args: &[i64]) -> Result<i64, String> {
            let &off = self
                .funcs
                .get(name)
                .ok_or_else(|| format!("no function {} in module", name))?;
            let p = unsafe { self.base.add(off) };
            macro_rules! call_as {
                ($($t:ty),*) => { unsafe {
                    let f: extern "C" fn($($t),*) -> i64 = std::mem::transmute(p);
                    #[allow(unused_variables, unused_mut)]
                    let mut it = args.iter().copied();
                    f($({ let _: $t; it.next().unwrap() }),*)
                }};
            }
            Ok(match args.len() {
                0 => call_as!(),
                1 => call_as!(i64),
                2 => call_as!(i64, i64),
                3 => call_as!(i64, i64, i64),
                4 => call_as!(i64, i64, i64, i64),
                5 => call_as!(i64, i64, i64, i64, i64),
                6 => call_as!(i64, i64, i64, i64, i64, i64),
                7 => call_as!(i64, i64, i64, i64, i64, i64, i64),
                8 => call_as!(i64, i64, i64, i64, i64, i64, i64, i64),
                n => return Err(format!("{} arguments not supported (x0-x7 carry 8)", n)),
            })
        }

        /// Call a function that returns two values. Our convention returns
        /// them in x0/x1, which is exactly where the C ABI puts a returned
        /// two-element aggregate — so a #[repr(C)] pair maps directly.
        /// (Three or more would need an SSA-generated shim; not yet.)
        pub fn call2(&self, name: &str, args: &[i64]) -> Result<(i64, i64), String> {
            #[repr(C)]
            struct Pair(i64, i64);
            let &off = self
                .funcs
                .get(name)
                .ok_or_else(|| format!("no function {} in module", name))?;
            let p = unsafe { self.base.add(off) };
            macro_rules! call_as {
                ($($t:ty),*) => { unsafe {
                    let f: extern "C" fn($($t),*) -> Pair = std::mem::transmute(p);
                    #[allow(unused_variables, unused_mut)]
                    let mut it = args.iter().copied();
                    f($({ let _: $t; it.next().unwrap() }),*)
                }};
            }
            let r = match args.len() {
                0 => call_as!(),
                1 => call_as!(i64),
                2 => call_as!(i64, i64),
                3 => call_as!(i64, i64, i64),
                4 => call_as!(i64, i64, i64, i64),
                5 => call_as!(i64, i64, i64, i64, i64),
                6 => call_as!(i64, i64, i64, i64, i64, i64),
                7 => call_as!(i64, i64, i64, i64, i64, i64, i64),
                8 => call_as!(i64, i64, i64, i64, i64, i64, i64, i64),
                n => return Err(format!("{} arguments not supported (x0-x7 carry 8)", n)),
            };
            Ok((r.0, r.1))
        }
    }
}

// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ssa::Repr;

    fn jit(src: &str) -> jit::JitCode {
        jit_on(src, &Platform::arm64())
    }

    fn jit_on(src: &str, platform: &Platform) -> jit::JitCode {
        let module = crate::ssa::parse(src).expect("parse");
        crate::ssa::verify(&module).expect("verify");
        let enc = Encoder::load("targets/arm64.encodings.json").expect("encodings");
        let compiled = compile_with(&module, &enc, platform).expect("compile");
        jit::JitCode::new(&compiled).expect("jit map")
    }

    /// what the harness sees: x0 normalized by the returned type (a w
    /// value's high half is don't-care)
    fn native_result(r: Repr, x0: i64) -> i64 {
        if r.container() == 32 {
            crate::opt::norm(r, x0 as u32 as i64)
        } else {
            x0
        }
    }

    #[test]
    fn i32_arithmetic_wraps() {
        let j = jit(r"
fn addmul(a: i32, b: i32) -> i32 {
entry:
    s: i32 = add a, b
    p: i32 = mul s, b
    ret p
}
");
        assert_eq!(native_result(Repr::S(32), j.call("addmul", &[3, 4]).unwrap()), 28);
        // i32 semantics: results wrap at 32 bits
        assert_eq!(
            native_result(Repr::S(32), j.call("addmul", &[0x7fffffff, 1]).unwrap()),
            i32::MIN as i64
        );
    }

    #[test]
    fn casts_of_negatives() {
        let j = jit(r"
fn half_ext(a: i32) -> i64 {
entry:
    w: i64 = conv a
    two: i64 = const 2
    h: i64 = div w, two
    ret h
}
");
        // a arrives as a 32-bit pattern; ext must recover the sign from bit 31
        assert_eq!(j.call("half_ext", &[0xfffffff6]).unwrap(), -5); // -10 / 2
        assert_eq!(j.call("half_ext", &[10]).unwrap(), 5);
    }

    #[test]
    fn i1_sign_extension() {
        let j = jit(r"
fn mask(a: i64, b: i64) -> i64 {
entry:
    lt: u1 = cmp.lt a, b
    s: i1 = cast lt
    m: i64 = conv s
    ret m
}
");
        assert_eq!(j.call("mask", &[1, 2]).unwrap(), -1);
        assert_eq!(j.call("mask", &[2, 1]).unwrap(), 0);
    }

    #[test]
    fn memory_swap() {
        let j = jit(r"
fn swap(p: ptr, q: ptr) {
entry:
    a: i64 = load p
    b: i64 = load q
    store b, p
    store a, q
    ret
}
");
        let mut a: i64 = 111;
        let mut b: i64 = 222;
        j.call("swap", &[&mut a as *mut i64 as i64, &mut b as *mut i64 as i64])
            .unwrap();
        assert_eq!((a, b), (222, 111));
    }

    #[test]
    fn i32_memory_and_ptradd() {
        let j = jit(r"
fn sum4(p: ptr) -> i32 {
entry:
    zero32: i32 = const 0
    zero: i64 = const 0
    jmp loop(zero, zero32)
loop(i: i64, acc: i32):
    four: i64 = const 4
    done: u1 = cmp.ge i, four
    br done, exit, body
body:
    off: i64 = mul i, four
    q: ptr = ptradd p, off
    v: i32 = load q
    acc2: i32 = add acc, v
    one: i64 = const 1
    i2: i64 = add i, one
    jmp loop(i2, acc2)
exit:
    ret acc
}
");
        let data: [i32; 4] = [10, 20, 30, 40];
        assert_eq!(j.call("sum4", &[data.as_ptr() as i64]).unwrap(), 100);
    }

    #[test]
    fn shifts_rems_unsigned() {
        let j = jit(r"
fn mix(a: u64, b: u64) -> u64 {
entry:
    sh: u64 = shl a, b
    r: u64 = rem sh, a
    x: u64 = xor r, b
    ret x
}
");
        // (7 << 3) = 56; 56 % 7 = 0; 0 ^ 3 = 3
        assert_eq!(j.call("mix", &[7, 3]).unwrap(), 3);
    }

    #[test]
    fn negative_iconst() {
        let j = jit(r"
fn neg() -> i64 {
entry:
    m: i64 = const -42
    ret m
}
");
        assert_eq!(j.call("neg", &[]).unwrap(), -42);
    }

    /// Every binary op and comparison on a set of narrow types, over every
    /// pair of values (or a dense sample for the wide ones), against the
    /// const-folder's N-bit model — the JIT and the model must agree bit
    /// for bit.
    #[test]
    fn narrow_types_exhaustive_against_model() {
        use crate::opt::{fold_bin, fold_cmp, norm};
        use crate::ssa::{BinOp, Cond};
        let ops = [
            ("add", BinOp::IAdd),
            ("sub", BinOp::ISub),
            ("mul", BinOp::IMul),
            ("div", BinOp::Div),
            ("rem", BinOp::Rem),
            ("and", BinOp::And),
            ("or", BinOp::Or),
            ("xor", BinOp::Xor),
            ("shl", BinOp::Shl),
            ("shr", BinOp::Shr),
        ];
        let conds = [
            ("eq", Cond::Eq),
            ("ne", Cond::Ne),
            ("lt", Cond::Lt),
            ("le", Cond::Le),
            ("gt", Cond::Gt),
            ("ge", Cond::Ge),
        ];
        for r in [
            Repr::S(1),
            Repr::U(1),
            Repr::S(3),
            Repr::U(3),
            Repr::S(5),
            Repr::U(5),
            Repr::S(6),
            Repr::U(6),
            Repr::S(8),
            Repr::U(8),
            Repr::S(12),
            Repr::U(12),
            Repr::S(32),
            Repr::U(32),
            Repr::S(33),
            Repr::U(40),
            Repr::S(64),
            Repr::U(64),
        ] {
            let ty = if r.signed() { format!("i{}", r.bits()) } else { format!("u{}", r.bits()) };
            let mut src = String::new();
            for (name, _) in &ops {
                src.push_str(&format!(
                    "fn {n}(a: {t}, b: {t}) -> {t} {{\nentry:\n    r: {t} = {n} a, b\n    ret r\n}}\n",
                    n = name,
                    t = ty
                ));
            }
            for (name, _) in &conds {
                src.push_str(&format!(
                    "fn c_{n}(a: {t}, b: {t}) -> u1 {{\nentry:\n    r: u1 = cmp.{n} a, b\n    ret r\n}}\n",
                    n = name,
                    t = ty
                ));
            }
            let j = jit(&src);
            // the value set: exhaustive up to 8 bits, else a spread of edges
            let vals: Vec<i64> = if r.bits() <= 8 {
                let n = 1i64 << r.bits();
                (0..n).map(|v| norm(r, v)).collect()
            } else {
                let n = r.bits();
                let mut v = vec![0i64, 1, 2, 3, 5, 7, -1, -2, -3, 100, -100, 12345, -12345];
                let half = 1i64 << (n - 1);
                v.extend([half, half.wrapping_sub(1), half.wrapping_add(1)]);
                if n < 64 {
                    v.extend([(1i64 << n) - 1, (1i64 << n) - 2, 1i64 << (n - 2)]);
                }
                v.extend([i64::MAX, i64::MIN, 0x5555_5555_5555_5555, -0x5555_5555_5555_5555]);
                v.into_iter().map(|v| norm(r, v)).collect()
            };
            for (name, op) in &ops {
                for &a in &vals {
                    for &b in &vals {
                        let Some(want) = fold_bin(*op, r, a, b) else {
                            continue; // division by zero / 64-bit MIN/-1: not modeled
                        };
                        let got = native_result(r, j.call(name, &[a, b]).unwrap());
                        assert_eq!(
                            got, want,
                            "{} {} {} {}: jit {} vs model {}",
                            ty, name, a, b, got, want
                        );
                    }
                }
            }
            for (name, cond) in &conds {
                for &a in &vals {
                    for &b in &vals {
                        let want = fold_cmp(*cond, r, a, b) as i64;
                        let got = native_result(Repr::U(1), j.call(&format!("c_{}", name), &[a, b]).unwrap());
                        assert_eq!(got, want, "{} cmp.{} {} {}", ty, name, a, b);
                    }
                }
            }
        }
    }

    /// ext / trunc / bitcast between many type pairs, against the model
    #[test]
    fn casts_exhaustive_against_model() {
        use crate::opt::norm;
        let types = [
            Repr::S(1),
            Repr::U(1),
            Repr::S(5),
            Repr::U(5),
            Repr::S(8),
            Repr::U(8),
            Repr::S(20),
            Repr::U(20),
            Repr::S(32),
            Repr::U(32),
            Repr::S(45),
            Repr::U(45),
            Repr::S(64),
            Repr::U(64),
        ];
        let name = |r: Repr| if r.signed() { format!("i{}", r.bits()) } else { format!("u{}", r.bits()) };
        let mut src = String::new();
        let mut cases = Vec::new();
        for &from in &types {
            for &to in &types {
                let op = if to.bits() == from.bits() { "cast" } else { "conv" };
                let fname = format!("{}_{}_{}", op, name(from), name(to));
                src.push_str(&format!(
                    "fn {f}(a: {s}) -> {d} {{\nentry:\n    r: {d} = {op} a\n    ret r\n}}\n",
                    f = fname,
                    s = name(from),
                    d = name(to),
                    op = op
                ));
                cases.push((fname, from, to));
            }
        }
        let j = jit(&src);
        let raw = [0i64, 1, 2, 7, -1, -2, 15, 16, 31, -16, 127, -128, 255, 1000, -1000, 0x7fff_ffff, -0x8000_0000, 0xffff_ffff, 1 << 40, -(1 << 40), i64::MAX, i64::MIN];
        for (fname, from, to) in &cases {
            for &v in &raw {
                let a = norm(*from, v); // callers pass canonical values
                let want = norm(*to, a);
                let got = native_result(*to, j.call(fname, &[a]).unwrap());
                assert_eq!(got, want, "{} of {}", fname, a);
            }
        }
    }

    #[test]
    fn packs_and_narrow_memory() {
        let j = jit(r"
type rgb = pack { r: u5, g: u6, b: u5 }
type mix = pack { s: i3, c: rgb, t: i9, flag: u1 }

fn mk(r: u5, g: u6, b: u5) -> rgb {
entry:
    c: rgb = pack r, g, b
    ret c
}
fn g(c: rgb) -> u6 {
entry:
    g: u6 = get c, g
    ret g
}
fn setg(c: rgb, g: u6) -> rgb {
entry:
    d: rgb = set c, g, g
    ret d
}
fn unpack_sum(c: rgb) -> u64 {
entry:
    r: u5, g: u6, b: u5 = unpack c
    r6: u64 = conv r
    g6: u64 = conv g
    b6: u64 = conv b
    x: u64 = add r6, g6
    y: u64 = add x, b6
    ret y
}
fn nested(s: i3, w: u16, t: i9, f: u1) -> (i64, i64) {
entry:
    c: rgb = cast w
    m: mix = pack s, c, t, f
    s2: i3 = get m, s
    t2: i9 = get m, t
    sw: i64 = conv s2
    tw: i64 = conv t2
    ret sw, tw
}
fn nested_bits(s: i3, w: u16, t: i9, f: u1) -> u64 {
entry:
    c: rgb = cast w
    m: mix = pack s, c, t, f
    c2: rgb = get m, c
    cw: u16 = cast c2
    f2: u1 = get m, flag
    cw64: u64 = conv cw
    f64: u64 = conv f2
    bits: u29 = cast m
    all: u64 = conv bits
    x: u64 = xor all, cw64
    y: u64 = xor x, f64
    ret y
}
fn bytes(p: ptr, v: i8) -> i64 {
entry:
    store v, p
    one: i64 = const 1
    q: ptr = ptradd p, one
    u: u8 = cast v
    store u, q
    a: i8 = load p
    b: u8 = load q
    aw: i64 = conv a
    bw: i64 = conv b
    r: i64 = sub aw, bw
    ret r
}
fn halves(p: ptr, v: i16) -> i64 {
entry:
    store v, p
    two: i64 = const 2
    q: ptr = ptradd p, two
    u: u16 = cast v
    store u, q
    a: i16 = load p
    b: u16 = load q
    aw: i64 = conv a
    bw: i64 = conv b
    r: i64 = sub aw, bw
    ret r
}
");
        let rgb = |r: i64, g: i64, b: i64| r | (g << 5) | (b << 11);
        assert_eq!(native_result(Repr::U(16), j.call("mk", &[31, 63, 1]).unwrap()), rgb(31, 63, 1));
        assert_eq!(native_result(Repr::U(6), j.call("g", &[rgb(9, 42, 3)]).unwrap()), 42);
        assert_eq!(native_result(Repr::U(16), j.call("setg", &[rgb(9, 42, 3), 7]).unwrap()), rgb(9, 7, 3));
        assert_eq!(j.call("unpack_sum", &[rgb(9, 42, 3)]).unwrap(), 54);
        // nested: s in bits 0-2, c in 3-18, t in 19-27, flag at 28
        assert_eq!(j.call2("nested", &[-3, rgb(1, 2, 3), -200, 1]).unwrap(), (-3, -200));
        let m = (5i64) | (rgb(1, 2, 3) << 3) | (((-200i64) & 0x1ff) << 19) | (1 << 28);
        assert_eq!(
            j.call("nested_bits", &[-3, rgb(1, 2, 3), -200, 1]).unwrap(),
            m ^ rgb(1, 2, 3) ^ 1
        );
        let mut buf = [0u8; 8];
        assert_eq!(j.call("bytes", &[buf.as_mut_ptr() as i64, -5]).unwrap(), -5 - 251);
        assert_eq!(buf[0], 0xfb);
        let mut buf = [0u8; 8];
        assert_eq!(j.call("halves", &[buf.as_mut_ptr() as i64, -5]).unwrap(), -5 - 65531);
        assert_eq!(buf[0], 0xfb);
        assert_eq!(buf[1], 0xff);
    }

    /// Round a finite, nonzero f64 to the nearest float(E, M) (ties to
    /// even), including subnormals and overflow to infinity. Used as the
    /// reference for formats the FPU doesn't have; the f64 sum of two such
    /// values is exact, so a single rounding is the correctly rounded sum.
    fn round_to(e: u32, m: u32, v: f64) -> u64 {
        let emax = (1u64 << e) - 1;
        let bias = (1i64 << (e - 1)) - 1;
        let bits = v.to_bits();
        let sign = bits >> 63;
        if v.is_nan() {
            return (emax << m) | (1u64 << (m - 1));
        }
        if v.is_infinite() {
            return (sign << (e + m)) | (emax << m);
        }
        if v == 0.0 {
            return sign << (e + m);
        }
        let e64 = ((bits >> 52) & 0x7ff) as i64;
        let frac = bits & ((1u64 << 52) - 1);
        // f64 subnormals don't arise from sums of narrow formats
        assert!(e64 != 0, "f64 subnormal in reference");
        let sig = frac | (1u64 << 52); // 53 bits
        let mut x = e64 - 1023 + bias; // target biased exponent
        // shift the 53-bit significand down to M + 1 bits (hidden + M),
        // further if the target is subnormal
        let mut shift = 52 - m as i64;
        if x < 1 {
            shift += 1 - x;
            x = 1;
        }
        let (mut mant, round, sticky) = if shift >= 64 {
            (0u64, false, sig != 0)
        } else if shift == 0 {
            (sig, false, false)
        } else {
            let dropped = sig & ((1u64 << shift) - 1);
            let half = 1u64 << (shift - 1);
            (sig >> shift, dropped & half != 0, dropped & (half - 1) != 0)
        };
        if round && (sticky || mant & 1 == 1) {
            mant += 1;
        }
        if mant >> (m + 1) != 0 {
            mant >>= 1;
            x += 1;
        }
        let (exp, mant) = if mant >> m != 0 {
            (x as u64, mant & ((1u64 << m) - 1))
        } else {
            (0, mant) // subnormal (x == 1)
        };
        if exp >= emax {
            return (sign << (e + m)) | (emax << m);
        }
        (sign << (e + m)) | (exp << m) | mant
    }

    fn to_f64(e: u32, m: u32, bits: u64) -> f64 {
        let emax = (1u64 << e) - 1;
        let bias = (1i64 << (e - 1)) - 1;
        let sign = if bits >> (e + m) & 1 == 1 { -1.0 } else { 1.0 };
        let exp = (bits >> m) & emax;
        let mant = bits & ((1u64 << m) - 1);
        if exp == emax {
            return if mant == 0 { sign * f64::INFINITY } else { f64::NAN };
        }
        if exp == 0 {
            return sign * (mant as f64) * 2f64.powi((1 - bias - m as i64) as i32);
        }
        sign * ((mant | (1u64 << m)) as f64) * 2f64.powi((exp as i64 - bias - m as i64) as i32)
    }

    /// exact reference: a finite value as sign, integer mantissa, and a
    /// power-of-two exponent (value = mant * 2^exp)
    fn parts(e: u32, m: u32, bits: u64) -> Option<(u64, u128, i64)> {
        let emax = (1u64 << e) - 1;
        let bias = (1i64 << (e - 1)) - 1;
        let sign = bits >> (e + m) & 1;
        let exp = (bits >> m) & emax;
        let mant = bits & ((1u64 << m) - 1);
        if exp == emax {
            return None;
        }
        if exp == 0 {
            Some((sign, mant as u128, 1 - bias - m as i64))
        } else {
            Some((sign, (mant | (1u64 << m)) as u128, exp as i64 - bias - m as i64))
        }
    }

    /// round sign * mant * 2^exp (sticky: some nonzero amount below mant's
    /// unit was dropped already) to float(E, M), nearest even
    fn round_parts(e: u32, m: u32, sign: u64, mut mant: u128, mut exp: i64, mut sticky: bool) -> u64 {
        let emax = (1u64 << e) - 1;
        let bias = (1i64 << (e - 1)) - 1;
        if mant == 0 {
            return sign << (e + m);
        }
        // bring mant to exactly M + 1 significant bits, collecting round/sticky
        let mut round = false;
        let top = 127 - mant.leading_zeros() as i64; // position of the top bit
        let mut shift = top - m as i64;
        // biased exponent of the M+1-bit significand's unit at bit M
        let mut x = exp + shift + m as i64 + bias;
        if x < 1 {
            shift += 1 - x;
            x = 1;
        }
        if shift > 0 {
            if shift >= 128 {
                sticky |= mant != 0;
                mant = 0;
                round = false;
            } else {
                let dropped = mant & ((1u128 << shift) - 1);
                let half = 1u128 << (shift - 1);
                round = dropped & half != 0;
                sticky |= dropped & (half - 1) != 0;
                mant >>= shift;
            }
        } else if shift < 0 {
            mant <<= -shift;
        }
        exp = 0;
        let _ = exp;
        if round && (sticky || mant & 1 == 1) {
            mant += 1;
        }
        if mant >> (m + 1) != 0 {
            mant >>= 1;
            x += 1;
        }
        let (fexp, fmant) = if mant >> m != 0 {
            (x as u64, (mant as u64) & ((1u64 << m) - 1))
        } else {
            (0, mant as u64)
        };
        if fexp >= emax {
            return (sign << (e + m)) | (emax << m);
        }
        (sign << (e + m)) | (fexp << m) | fmant
    }

    fn isqrt128(n: u128) -> u128 {
        if n < 2 {
            return n;
        }
        let mut x = 1u128 << ((128 - n.leading_zeros()).div_ceil(2));
        loop {
            let y = (x + n / x) / 2;
            if y >= x {
                return x;
            }
            x = y;
        }
    }

    /// the exact square root in float(E, M), for the narrow formats
    fn ref_sqrt(e: u32, m: u32, a: u64) -> u64 {
        let nan = ((1u64 << e) - 1) << m | (1u64 << (m - 1));
        if is_nan_bits(e, m, a) {
            return nan;
        }
        let sa = a >> (e + m) & 1;
        match parts(e, m, a) {
            None => {
                if sa == 1 {
                    nan
                } else {
                    a
                }
            }
            Some((_, 0, _)) => a, // +-0
            Some(_) if sa == 1 => nan,
            Some((_, mant, exp)) => {
                // sqrt(mant * 2^exp): make the exponent even, then an integer
                // root of mant << 2k with a sticky for the remainder
                let (mant, exp) = if exp & 1 != 0 { (mant << 1, exp - 1) } else { (mant, exp) };
                let k = 50;
                let n = mant << (2 * k);
                let r = isqrt128(n);
                let sticky = r * r != n;
                round_parts(e, m, 0, r, exp / 2 - k, sticky)
            }
        }
    }

    /// the exact result of a op b in float(E, M), for the narrow formats
    fn ref_op(e: u32, m: u32, op: &str, a: u64, b: u64) -> u64 {
        let nan = ((1u64 << e) - 1) << m | (1u64 << (m - 1));
        let inf = |s: u64| (s << (e + m)) | (((1u64 << e) - 1) << m);
        if is_nan_bits(e, m, a) || is_nan_bits(e, m, b) {
            return nan;
        }
        let pa = parts(e, m, a);
        let pb = parts(e, m, b);
        let sa = a >> (e + m) & 1;
        let sb = b >> (e + m) & 1;
        match op {
            "add" | "sub" => {
                let sb = if op == "sub" { sb ^ 1 } else { sb };
                let b = if op == "sub" { b ^ (1u64 << (e + m)) } else { b };
                match (pa, pb) {
                    (None, None) => {
                        if sa == sb {
                            inf(sa)
                        } else {
                            nan
                        }
                    }
                    (None, _) => inf(sa),
                    (_, None) => inf(sb),
                    (Some(_), Some(_)) => {
                        // exact in f64 for these formats, single rounding after
                        let v = to_f64(e, m, a) + to_f64(e, m, b);
                        if v == 0.0 {
                            let (za, zb) = (pa.unwrap().1 == 0, pb.unwrap().1 == 0);
                            let s = if za && zb { sa & sb } else { 0 };
                            return s << (e + m);
                        }
                        round_to(e, m, v)
                    }
                }
            }
            "mul" => {
                let s = sa ^ sb;
                match (pa, pb) {
                    (None, Some((_, 0, _))) | (Some((_, 0, _)), None) => nan,
                    (None, _) | (_, None) => inf(s),
                    (Some((_, ma, xa)), Some((_, mb, xb))) => round_parts(e, m, s, ma * mb, xa + xb, false),
                }
            }
            "div" => {
                let s = sa ^ sb;
                match (pa, pb) {
                    (None, None) => nan,
                    (Some((_, 0, _)), Some((_, 0, _))) => nan,
                    (None, _) | (_, Some((_, 0, _))) => inf(s),
                    (_, None) | (Some((_, 0, _)), _) => s << (e + m),
                    (Some((_, ma, xa)), Some((_, mb, xb))) => {
                        // q = ma * 2^100 / mb with a sticky for the remainder
                        let num = ma << 100;
                        let q = num / mb;
                        let sticky = num % mb != 0;
                        round_parts(e, m, s, q, xa - xb - 100, sticky)
                    }
                }
            }
            _ => unreachable!(),
        }
    }

    fn is_nan_bits(e: u32, m: u32, bits: u64) -> bool {
        let emax = (1u64 << e) - 1;
        (bits >> m) & emax == emax && bits & ((1u64 << m) - 1) != 0
    }

    /// The generic softfloat add against the FPU (f32, f64) and against
    /// an independent reference (fp8 exhaustively, fp16 and bf16 densely) —
    /// once compiled as the library (every fadd a call into SSA) and once
    /// on the platform (fadd32/fadd64 as the hardware instruction).
    #[test]
    fn softfloat_add_matches_hardware_and_reference() {
        let src = crate::ssa::with_prelude(include_str!("../suite/float.ssa"));
        let src = src.as_str();
        let module = crate::ssa::parse(src).expect("parse");
        let enc = Encoder::load("targets/arm64.encodings.json").expect("encodings");
        let soft = compile_with(&module, &enc, &Platform::none()).expect("compile");
        let hard = compile_with(&module, &enc, &Platform::arm64()).expect("compile");
        assert_ne!(soft.code, hard.code, "the platform must change the code for fadd32/fadd64");
        // fadd32 on the platform is four instructions plus frame: much
        // smaller than the library instance
        let size = |c: &Compiled, n: &str| {
            let mut offs: Vec<usize> = c.funcs.values().copied().collect();
            offs.sort();
            let at = c.funcs[n];
            offs.iter().find(|&&o| o > at).map(|&o| o - at).unwrap_or(c.code.len() - at)
        };
        assert!(size(&hard, "add_8_23_0") < size(&soft, "add_8_23_0") / 4, "{} vs {}", size(&hard, "add_8_23_0"), size(&soft, "add_8_23_0"));
        for platform in [Platform::none(), Platform::arm64()] {
            softfloat_check(&jit_on(src, &platform));
        }
    }

    fn softfloat_check(j: &jit::JitCode) {
        let mask = |w: u32| if w == 64 { u64::MAX } else { (1u64 << w) - 1 };
        let mut seed = 0x9e3779b97f4a7c15u64;
        let mut rnd = || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        // interesting bit patterns for a format of width w: specials,
        // boundaries, and random values with random exponents
        let patterns = |e: u32, m: u32, n: usize, rnd: &mut dyn FnMut() -> u64| -> Vec<u64> {
            let w = e + m + 1;
            let emax = (1u64 << e) - 1;
            let mut v = vec![
                0,
                1u64 << (w - 1),                    // -0
                1,                                  // min subnormal
                (1u64 << m) - 1,                    // max subnormal
                1u64 << m,                          // min normal
                ((emax - 1) << m) | ((1u64 << m) - 1), // max finite
                emax << m,                          // +inf
                (emax << m) | (1u64 << (w - 1)),    // -inf
                (emax << m) | 1,                    // NaN
                ((emax / 2) << m),                  // 1.0
            ];
            for _ in 0..n {
                let r = rnd();
                v.push(r & mask(w));
                // values near each other in magnitude cancel interestingly
                let x = v[v.len() - 1];
                v.push((x ^ (1u64 << (w - 1)) ^ (r >> 40 & 3)) & mask(w));
            }
            v
        };
        let ops = ["add", "sub", "mul", "div"];
        // --- f32 and f64: the FPU is the oracle ---
        let f32op = |op: &str, a: f32, b: f32| match op {
            "add" => a + b,
            "sub" => a - b,
            "mul" => a * b,
            _ => a / b,
        };
        let f64op = |op: &str, a: f64, b: f64| match op {
            "add" => a + b,
            "sub" => a - b,
            "mul" => a * b,
            _ => a / b,
        };
        let vals = patterns(8, 23, 160, &mut rnd);
        for &a in &vals {
            let want = f32::from_bits(a as u32).sqrt().to_bits() as u64;
            let got = native_result(Repr::U(32), j.call("fsqrt32", &[a as i64]).unwrap()) as u64;
            if is_nan_bits(8, 23, want) {
                assert!(is_nan_bits(8, 23, got), "f32 sqrt {:#x}: got {:#x}, want NaN", a, got);
            } else {
                assert_eq!(got, want, "f32 sqrt {:#x}", a);
            }
        }
        for op in ops {
            let name = format!("f{}32", op);
            for &a in &vals {
                for &b in &vals {
                    let want = f32op(op, f32::from_bits(a as u32), f32::from_bits(b as u32)).to_bits() as u64;
                    let got = native_result(Repr::U(32), j.call(&name, &[a as i64, b as i64]).unwrap()) as u64;
                    if is_nan_bits(8, 23, want) {
                        assert!(is_nan_bits(8, 23, got), "f32 {:#x} {} {:#x}: got {:#x}, want NaN", a, op, b, got);
                    } else {
                        assert_eq!(got, want, "f32 {:#x} {} {:#x}", a, op, b);
                    }
                }
            }
        }
        let vals = patterns(11, 52, 120, &mut rnd);
        for &a in &vals {
            let want = f64::from_bits(a).sqrt().to_bits();
            let got = j.call("fsqrt64", &[a as i64]).unwrap() as u64;
            if is_nan_bits(11, 52, want) {
                assert!(is_nan_bits(11, 52, got), "f64 sqrt {:#x}: got {:#x}, want NaN", a, got);
            } else {
                assert_eq!(got, want, "f64 sqrt {:#x}", a);
            }
        }
        for op in ops {
            let name = format!("f{}64", op);
            for &a in &vals {
                for &b in &vals {
                    let want = f64op(op, f64::from_bits(a), f64::from_bits(b)).to_bits();
                    let got = j.call(&name, &[a as i64, b as i64]).unwrap() as u64;
                    if is_nan_bits(11, 52, want) {
                        assert!(is_nan_bits(11, 52, got), "f64 {:#x} {} {:#x}: got {:#x}, want NaN", a, op, b, got);
                    } else {
                        assert_eq!(got, want, "f64 {:#x} {} {:#x}", a, op, b);
                    }
                }
            }
        }
        let check = |name: &str, arg: i64, want: u64, r: Repr, e: u32, m: u32| {
            let got = native_result(r, j.call(name, &[arg]).unwrap()) as u64;
            if e > 0 && is_nan_bits(e, m, want) {
                assert!(is_nan_bits(e, m, got), "{}({:#x}): got {:#x}, want NaN", name, arg, got);
            } else {
                assert_eq!(got, want, "{}({:#x})", name, arg);
            }
        };
        // --- fma against the FPU (f32, f64) and an exact i128 reference
        // (f16); min/max against the IEEE rules ---
        let v32 = patterns(8, 23, 40, &mut rnd);
        for &a in &v32 {
            for &b in &v32 {
                for &c in &v32 {
                    let (fa, fb, fc) = (f32::from_bits(a as u32), f32::from_bits(b as u32), f32::from_bits(c as u32));
                    let want = fa.mul_add(fb, fc).to_bits() as u64;
                    let got = native_result(Repr::U(32), j.call("fma32", &[a as i64, b as i64, c as i64]).unwrap()) as u64;
                    if is_nan_bits(8, 23, want) {
                        assert!(is_nan_bits(8, 23, got), "fma32 {:#x} {:#x} {:#x}: got {:#x}, want NaN", a, b, c, got);
                    } else {
                        assert_eq!(got, want, "fma32 {:#x} {:#x} {:#x}", a, b, c);
                    }
                }
            }
        }
        let v64 = patterns(11, 52, 30, &mut rnd);
        for &a in &v64 {
            for &b in &v64 {
                for &c in &v64 {
                    let (fa, fb, fc) = (f64::from_bits(a), f64::from_bits(b), f64::from_bits(c));
                    let want = fa.mul_add(fb, fc).to_bits();
                    let got = j.call("fma64", &[a as i64, b as i64, c as i64]).unwrap() as u64;
                    if is_nan_bits(11, 52, want) {
                        assert!(is_nan_bits(11, 52, got), "fma64 {:#x} {:#x} {:#x}: got {:#x}, want NaN", a, b, c, got);
                    } else {
                        assert_eq!(got, want, "fma64 {:#x} {:#x} {:#x}", a, b, c);
                    }
                }
            }
        }
        // f16: a * b + c exactly in i128 (the exponent range keeps every
        // alignment under 128 bits), rounded once
        let ref_fma16 = |a: u64, b: u64, c: u64| -> u64 {
            let (e, m) = (5u32, 10u32);
            let nan = ((1u64 << e) - 1) << m | (1u64 << (m - 1));
            if is_nan_bits(e, m, a) || is_nan_bits(e, m, b) || is_nan_bits(e, m, c) {
                return nan;
            }
            let (sa, sb, sc) = (a >> 15 & 1, b >> 15 & 1, c >> 15 & 1);
            let sp = sa ^ sb;
            let inf = |s: u64| (s << 15) | (0x1f << 10);
            match (parts(e, m, a), parts(e, m, b), parts(e, m, c)) {
                (None, Some((_, 0, _)), _) | (Some((_, 0, _)), None, _) => nan,
                (None, _, pc) | (_, None, pc) => {
                    if pc.is_none() && sc != sp {
                        nan
                    } else {
                        inf(sp)
                    }
                }
                (_, _, None) => c,
                (Some((_, ma, xa)), Some((_, mb, xb)), Some((_, mc, xc))) => {
                    let (pm, px) = (ma as i128 * mb as i128, xa + xb);
                    if pm == 0 {
                        return if mc == 0 { (sp & sc) << 15 } else { c };
                    }
                    let base = px.min(xc);
                    let p = (pm << (px - base)) * if sp == 1 { -1 } else { 1 };
                    let q = ((mc as i128) << (xc - base)) * if sc == 1 { -1 } else { 1 };
                    let sum = p + q;
                    if sum == 0 {
                        return 0;
                    }
                    round_parts(e, m, (sum < 0) as u64, sum.unsigned_abs(), base, false)
                }
            }
        };
        let v16 = patterns(5, 10, 40, &mut rnd);
        for &a in &v16 {
            for &b in &v16 {
                for &c in &v16 {
                    let want = ref_fma16(a, b, c);
                    let got = native_result(Repr::U(16), j.call("fma16", &[a as i64, b as i64, c as i64]).unwrap()) as u64;
                    if is_nan_bits(5, 10, want) {
                        assert!(is_nan_bits(5, 10, got), "fma16 {:#x} {:#x} {:#x}: got {:#x}, want NaN", a, b, c, got);
                    } else {
                        assert_eq!(got, want, "fma16 {:#x} {:#x} {:#x}", a, b, c);
                    }
                }
            }
        }
        // min/max: IEEE minimum/maximum
        let minmax = |is_min: bool, a: u64, b: u64, e: u32, m: u32| -> u64 {
            let nan = ((1u64 << e) - 1) << m | (1u64 << (m - 1));
            if is_nan_bits(e, m, a) || is_nan_bits(e, m, b) {
                return nan;
            }
            let (fa, fb) = (to_f64(e, m, a), to_f64(e, m, b));
            let key = |f: f64, bits: u64| (f, -((bits >> (e + m) & 1) as i64) as f64);
            let (ka, kb) = (key(fa, a), key(fb, b));
            if ka == kb {
                return a;
            }
            if (ka < kb) == is_min {
                a
            } else {
                b
            }
        };
        for &a in &v32 {
            for &b in &v32 {
                for (name, is_min) in [("min32", true), ("max32", false)] {
                    let want = minmax(is_min, a, b, 8, 23);
                    let got = native_result(Repr::U(32), j.call(name, &[a as i64, b as i64]).unwrap()) as u64;
                    if is_nan_bits(8, 23, want) {
                        assert!(is_nan_bits(8, 23, got), "{} {:#x} {:#x}", name, a, b);
                    } else {
                        assert_eq!(got, want, "{} {:#x} {:#x}", name, a, b);
                    }
                }
            }
        }
        for a in 0..256u64 {
            for b in 0..256u64 {
                let want = minmax(true, a, b, 4, 3);
                let got = native_result(Repr::U(8), j.call("min8", &[a as i64, b as i64]).unwrap()) as u64;
                if is_nan_bits(4, 3, want) {
                    assert!(is_nan_bits(4, 3, got), "min8 {:#x} {:#x}", a, b);
                } else {
                    assert_eq!(got, want, "min8 {:#x} {:#x}", a, b);
                }
            }
        }

        // --- comparisons, neg, abs: f32/f64 against Rust, fp8 exhaustively
        // against the exact values ---
        let conds = ["eq", "ne", "lt", "le", "gt", "ge"];
        let judge = |c: &str, o: Option<std::cmp::Ordering>| -> u64 {
            use std::cmp::Ordering::*;
            (match (c, o) {
                ("eq", Some(Equal)) | ("le", Some(Equal)) | ("ge", Some(Equal)) => true,
                ("lt", Some(Less)) | ("le", Some(Less)) => true,
                ("gt", Some(Greater)) | ("ge", Some(Greater)) => true,
                ("ne", o) => o != Some(Equal),
                _ => false,
            }) as u64
        };
        let v32 = patterns(8, 23, 60, &mut rnd);
        for &a in &v32 {
            let fa = f32::from_bits(a as u32);
            check("neg32", a as i64, (-fa).to_bits() as u64, Repr::U(32), 8, 23);
            check("abs32", a as i64, fa.abs().to_bits() as u64, Repr::U(32), 8, 23);
            for &b in &v32 {
                let fb = f32::from_bits(b as u32);
                for c in conds {
                    let want = judge(c, fa.partial_cmp(&fb));
                    let got = native_result(Repr::U(1), j.call(&format!("{}32", c), &[a as i64, b as i64]).unwrap()) as u64;
                    assert_eq!(got, want, "f32 {:#x} {} {:#x}", a, c, b);
                }
            }
        }
        let v64 = patterns(11, 52, 60, &mut rnd);
        for &a in &v64 {
            let fa = f64::from_bits(a);
            check("abs64", a as i64, fa.abs().to_bits(), Repr::U(64), 11, 52);
            for &b in &v64 {
                let fb = f64::from_bits(b);
                for c in ["lt", "ge", "eq"] {
                    let want = judge(c, fa.partial_cmp(&fb));
                    let got = native_result(Repr::U(1), j.call(&format!("{}64", c), &[a as i64, b as i64]).unwrap()) as u64;
                    assert_eq!(got, want, "f64 {:#x} {} {:#x}", a, c, b);
                }
            }
        }
        for a in 0..256u64 {
            for b in 0..256u64 {
                let (fa, fb) = (to_f64(4, 3, a), to_f64(4, 3, b));
                for c in ["lt", "gt"] {
                    let want = judge(c, fa.partial_cmp(&fb));
                    let got = native_result(Repr::U(1), j.call(&format!("{}8", c), &[a as i64, b as i64]).unwrap()) as u64;
                    assert_eq!(got, want, "fp8 {:#x} {} {:#x}", a, c, b);
                }
            }
        }

        // --- conversions: f32/f64 and the integer kinds against Rust's `as`
        // (round to nearest in, truncate-saturate-NaN-to-0 out) ---
        let v32 = patterns(8, 23, 200, &mut rnd);
        let v64 = patterns(11, 52, 200, &mut rnd);
        let ints: Vec<i64> = {
            let mut v = vec![0i64, 1, -1, 7, -7, 16777216, 16777217, 16777219, -16777217, i32::MAX as i64, i32::MIN as i64, u32::MAX as i64, 1 << 40, -(1 << 40), (1 << 53) + 1, i64::MAX, i64::MIN, -1i64 << 62];
            for _ in 0..100 {
                v.push(rnd() as i64);
                v.push((rnd() >> (rnd() % 64)) as i64);
            }
            v
        };
        for &a in &v32 {
            let f = f32::from_bits(a as u32);
            check("f32tof64", a as i64, (f as f64).to_bits(), Repr::U(64), 11, 52);
            check("f32toi32", a as i64, (f as i32) as i64 as u64, Repr::S(32), 0, 0);
            check("f32tou32", a as i64, (f as u32) as u64, Repr::U(32), 0, 0);
            check("f32toi64", a as i64, (f as i64) as u64, Repr::S(64), 0, 0);
            check("f32tof16", a as i64, round_to(5, 10, f as f64), Repr::U(16), 5, 10);
            check("f32tof8", a as i64, round_to(4, 3, f as f64), Repr::U(8), 4, 3);
        }
        for &a in &v64 {
            let f = f64::from_bits(a);
            check("f64tof32", a as i64, (f as f32).to_bits() as u64, Repr::U(32), 8, 23);
            check("f64toi64", a as i64, (f as i64) as u64, Repr::S(64), 0, 0);
            check("f64tou64", a as i64, f as u64, Repr::U(64), 0, 0);
        }
        for &i in &ints {
            check("i32tof32", i as i32 as i64, ((i as i32) as f32).to_bits() as u64, Repr::U(32), 8, 23);
            check("u32tof32", i as u32 as i64, ((i as u32) as f32).to_bits() as u64, Repr::U(32), 8, 23);
            check("i64tof32", i, (i as f32).to_bits() as u64, Repr::U(32), 8, 23);
            check("u64tof64", i, ((i as u64) as f64).to_bits(), Repr::U(64), 11, 52);
            check("i32tof64", i as i32 as i64, ((i as i32) as f64).to_bits(), Repr::U(64), 11, 52);
            check("i64tof64", i, (i as f64).to_bits(), Repr::U(64), 11, 52);
            check("i32tof16", i as i32 as i64, round_to(5, 10, (i as i32) as f64), Repr::U(16), 5, 10);
        }
        // fp16 -> f32 exactly, fp16 -> i32 by truncation, over every f16
        for a in 0..1u64 << 16 {
            let v = to_f64(5, 10, a);
            check("f16tof32", a as i64, round_to(8, 23, v), Repr::U(32), 8, 23);
            let want = if v.is_nan() { 0 } else { (v as i32) as i64 as u64 };
            check("f16toi32", a as i64, want, Repr::S(32), 0, 0);
        }
        for a in 0..1u64 << 8 {
            check("f8tof32", a as i64, round_to(8, 23, to_f64(4, 3, a)), Repr::U(32), 8, 23);
        }

        // --- fp8 exhaustive, fp16 / bf16 dense: the exact reference ---
        for (suffix, e, m, n) in [("8", 4u32, 3u32, 0usize), ("16", 5, 10, 200), ("b16", 8, 7, 200)] {
            let w = e + m + 1;
            let vals: Vec<u64> = if n == 0 {
                (0..1u64 << w).collect()
            } else {
                patterns(e, m, n, &mut rnd)
            };
            let name = format!("fsqrt{}", suffix);
            for &a in &vals {
                let want = ref_sqrt(e, m, a);
                let got = native_result(Repr::U(w), j.call(&name, &[a as i64]).unwrap()) as u64;
                if is_nan_bits(e, m, want) {
                    assert!(is_nan_bits(e, m, got), "{} {:#x}: got {:#x}, want NaN", name, a, got);
                } else {
                    assert_eq!(got, want, "{} {:#x}", name, a);
                }
            }
            for op in ops {
                let name = format!("f{}{}", op, suffix);
                if j.call(&name, &[0, 0]).is_err() {
                    continue; // bf16 has add and mul only
                }
                for &a in &vals {
                    for &b in &vals {
                        let want = ref_op(e, m, op, a, b);
                        let got = native_result(Repr::U(w), j.call(&name, &[a as i64, b as i64]).unwrap()) as u64;
                        if is_nan_bits(e, m, want) {
                            assert!(is_nan_bits(e, m, got), "{} {:#x} {} {:#x}: got {:#x}, want NaN", name, a, op, b, got);
                        } else {
                            assert_eq!(got, want, "{} {:#x} {} {:#x}", name, a, op, b);
                        }
                    }
                }
            }
        }
    }

    /// The platform decides per width: on arm64, `add` on f32 is the FPU
    /// (one fadd, no call) while `add` on f16 is the SSA library (a call,
    /// no fadd) — and both are right.
    #[test]
    fn platform_mixes_native_f32_with_emulated_f16() {
        let lib = crate::ssa::with_prelude(include_str!("../suite/float.ssa"));
        let src = format!(
            "{}\nfn sum32(a: f32, b: f32) -> f32 {{\n    r: f32 = add a, b\n    ret r\n}}\nfn sum16(a: f16, b: f16) -> f16 {{\n    r: f16 = add a, b\n    ret r\n}}\n",
            lib
        );
        let module = crate::ssa::parse(&src).expect("parse");
        crate::ssa::verify(&module).expect("verify");
        let enc = Encoder::load("targets/arm64.encodings.json").expect("encodings");
        let compiled = compile_with(&module, &enc, &Platform::arm64()).expect("compile");
        // the words of one function
        let words = |name: &str| -> Vec<u32> {
            let mut offs: Vec<usize> = compiled.funcs.values().copied().collect();
            offs.sort();
            let at = compiled.funcs[name];
            let end = offs.iter().find(|&&o| o > at).copied().unwrap_or(compiled.code.len());
            (at..end)
                .step_by(4)
                .map(|i| u32::from_le_bytes(compiled.code[i..i + 4].try_into().unwrap()))
                .collect()
        };
        // fadd {s}, {s}, {s} as learned: fixed 0x1e202800, registers in
        // bits 0-4, 5-9, 16-20; bl is fixed 0x94000000 over a 26-bit offset
        let is_fadd_s = |w: u32| w & 0xffe0_fc00 == 0x1e20_2800;
        let is_bl = |w: u32| w & 0xfc00_0000 == 0x9400_0000;
        let w32 = words("sum32");
        let w16 = words("sum16");
        assert!(w32.iter().any(|&w| is_fadd_s(w)), "sum32 should use the FPU: {:x?}", w32);
        assert!(!w32.iter().any(|&w| is_bl(w)), "sum32 should not call: {:x?}", w32);
        assert!(w16.iter().any(|&w| is_bl(w)), "sum16 should call the library: {:x?}", w16);
        assert!(!w16.iter().any(|&w| is_fadd_s(w)), "sum16 should not use the FPU: {:x?}", w16);
        // the dispatched call reuses the named instantiation from the library
        let sum16 = module.func("sum16").unwrap();
        let callee = sum16.blocks.iter().flat_map(|b| &b.insts).find_map(|i| match i {
            crate::ssa::Inst::Call { callee, .. } => Some(callee.clone()),
            _ => None,
        });
        assert_eq!(callee.as_deref(), Some("add_5_10_0"));
        // and both are right: f32 against the FPU, f16 against the reference
        let j = jit::JitCode::new(&compiled).expect("jit");
        for (a, b) in [(1.0f32, 2.0f32), (0.1, 0.2), (1.0, 1e-8), (3.0, -1.0), (f32::MAX, f32::MAX)] {
            let want = (a + b).to_bits() as i64;
            let got = native_result(Repr::U(32), j.call("sum32", &[a.to_bits() as i64, b.to_bits() as i64]).unwrap());
            assert_eq!(got, want, "{} + {}", a, b);
        }
        for (a, b) in [(0x3c00u64, 0x4000u64), (0x3c00, 0x3c00), (0x0001, 0x0001), (0x7bff, 0x7bff), (0x3c00, 0xbc00)] {
            let want = round_to(5, 10, to_f64(5, 10, a) + to_f64(5, 10, b));
            let got = native_result(Repr::U(16), j.call("sum16", &[a as i64, b as i64]).unwrap()) as u64;
            assert_eq!(got, want, "f16 {:#x} + {:#x}", a, b);
        }
    }

    /// fixed(8, 8) against an exact model: mul and div truncate toward
    /// zero, div by zero saturates, add wraps
    #[test]
    fn fixed_point_matches_model() {
        let src = crate::ssa::with_prelude(include_str!("../suite/fixed.ssa"));
        let j = jit(&src);
        let mut seed = 0x1234_5678_9abc_def1u64;
        let mut rnd = || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        let mut vals: Vec<i16> = vec![0, 1, -1, 256, -256, 384, -384, 127, -128, i16::MAX, i16::MIN, 0x7f00, -0x7f00, 3, -3];
        for _ in 0..150 {
            vals.push(rnd() as i16);
        }
        let as_bits = |v: i16| v as u16 as i64;
        let back = |x: i64| x as u16 as i16;
        for &a in &vals {
            for &b in &vals {
                let want_add = a.wrapping_add(b);
                let got = back(native_result(Repr::U(16), j.call("fadd88", &[as_bits(a), as_bits(b)]).unwrap()));
                assert_eq!(got, want_add, "fadd88 {} {}", a, b);
                // truncating mul: (a * b) / 256 toward zero, wrapped to 16 bits
                let p = a as i64 * b as i64;
                let want_mul = (p / 256) as i16; // Rust's / truncates toward zero
                let got = back(native_result(Repr::U(16), j.call("fmul88", &[as_bits(a), as_bits(b)]).unwrap()));
                assert_eq!(got, want_mul, "fmul88 {} {}", a, b);
                let want_div = if b == 0 {
                    if a == 0 {
                        0
                    } else if a < 0 {
                        i16::MIN
                    } else {
                        i16::MAX
                    }
                } else {
                    ((a as i64 * 256) / b as i64) as i16
                };
                let got = back(native_result(Repr::U(16), j.call("fdiv88", &[as_bits(a), as_bits(b)]).unwrap()));
                assert_eq!(got, want_div, "fdiv88 {} {}", a, b);
            }
            // conversions: to int floors; to float is exact; from float truncates toward zero
            let want_int = (a as i32) >> 8;
            assert_eq!(native_result(Repr::S(32), j.call("toint88", &[as_bits(a)]).unwrap()) as i32, want_int, "toint88 {}", a);
            let f = a as f32 / 256.0;
            assert_eq!(native_result(Repr::U(32), j.call("tof32", &[as_bits(a)]).unwrap()) as u32, f.to_bits(), "tof32 {}", a);
            let want_back = (f * 256.0) as i32 as i16;
            assert_eq!(back(native_result(Repr::U(16), j.call("fromf32", &[f.to_bits() as i64]).unwrap())), want_back, "fromf32 {}", a);
        }
    }

    /// unit(8) and sunit(8) against their models, exhaustively
    #[test]
    fn unit_types_match_model() {
        let src = crate::ssa::with_prelude(include_str!("../suite/unit.ssa"));
        let j = jit(&src);
        let max = 255i64;
        for a in 0..256i64 {
            for b in 0..256i64 {
                let call = |name: &str| native_result(Repr::U(8), j.call(name, &[a, b]).unwrap());
                assert_eq!(call("umul8"), (a * b + max / 2) / max, "umul8 {} {}", a, b);
                assert_eq!(call("uadd8"), (a + b).min(max), "uadd8 {} {}", a, b);
                assert_eq!(call("usub8"), (a - b).max(0), "usub8 {} {}", a, b);
                let want_div = if b == 0 {
                    if a == 0 { 0 } else { max }
                } else if a >= b {
                    max
                } else {
                    (a * max + b / 2) / b
                };
                assert_eq!(call("udiv8"), want_div, "udiv8 {} {}", a, b);
            }
        }
        let m = 127i64;
        let clamp = |x: i64| x.max(-m).min(m);
        for a in -128..128i64 {
            for b in -128..128i64 {
                let bits = |v: i64| v as u8 as i64;
                let call = |name: &str| native_result(Repr::U(8), j.call(name, &[bits(a), bits(b)]).unwrap()) as u8 as i8 as i64;
                assert_eq!(call("sadd8"), clamp(a + b), "sadd8 {} {}", a, b);
                let s = (a < 0) != (b < 0);
                let q = ((a.abs() * b.abs() + m / 2) / m).min(m);
                assert_eq!(call("smul8"), clamp(if s { -q } else { q }), "smul8 {} {}", a, b);
                let want_div = if a == 0 {
                    0
                } else {
                    let q = if b == 0 || a.abs() >= b.abs() { m } else { (a.abs() * m + b.abs() / 2) / b.abs() };
                    clamp(if s { -q } else { q })
                };
                assert_eq!(call("sdiv8"), want_div, "sdiv8 {} {}", a, b);
            }
            let neg = native_result(Repr::U(8), j.call("sneg8", &[a as u8 as i64]).unwrap()) as u8 as i8 as i64;
            assert_eq!(neg, clamp(-a), "sneg8 {}", a);
            let f = native_result(Repr::U(32), j.call("stof32", &[a as u8 as i64]).unwrap()) as u32;
            assert_eq!(f, (a as f32 / 127.0).to_bits(), "stof32 {}", a);
        }
        for a in 0..256i64 {
            let f = native_result(Repr::U(32), j.call("utof32", &[a]).unwrap()) as u32;
            assert_eq!(f, (a as f32 / 255.0).to_bits(), "utof32 {}", a);
            let back = native_result(Repr::U(8), j.call("ufromf32", &[f as i64]).unwrap());
            assert_eq!(back, a, "ufromf32(utof32({}))", a);
        }
    }

    /// rational(8, 8) against an exact model: reduced fractions, NaR on a
    /// zero denominator, and halved down when the exact answer does not fit
    #[test]
    fn rationals_match_model() {
        let src = crate::ssa::with_prelude(include_str!("../suite/rational.ssa"));
        let j = jit(&src);
        fn gcd(a: i64, b: i64) -> i64 {
            if b == 0 { a } else { gcd(b, a % b) }
        }
        // the library's rule for a result n/d
        let fit = |n: i64, d: i64| -> (i64, i64) {
            if d == 0 {
                return (0, 0);
            }
            if n == 0 {
                return (0, 1);
            }
            let (neg, mut m, mut d) = (n < 0, n.abs(), d);
            let g = gcd(m, d);
            m /= g;
            d /= g;
            while m > 127 || d > 255 {
                m >>= 1;
                d >>= 1;
                if d == 0 {
                    d = 1;
                }
            }
            let g = gcd(m, d);
            (if neg { -(m / g) } else { m / g }, d / g)
        };
        let enc = |n: i64, d: i64| ((n as u8 as i64) | (d << 8)) as i64;
        let dec = |bits: i64| ((bits as u8 as i8) as i64, (bits >> 8) & 0xff);
        let mut seed = 0xfeed_beef_cafe_f00du64;
        let mut rnd = || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        // values: reduced fractions, plus a few unreduced and NaR
        let mut vals: Vec<(i64, i64)> = vec![(0, 1), (1, 1), (-1, 1), (1, 2), (-1, 2), (1, 3), (2, 3), (127, 1), (-128, 1), (1, 255), (127, 255), (2, 4), (0, 0), (5, 0)];
        for _ in 0..120 {
            let n = (rnd() % 256) as i64 - 128;
            let d = (rnd() % 255) as i64 + 1;
            vals.push((n, d));
        }
        for &(an, ad) in &vals {
            for &(bn, bd) in &vals {
                let call = |name: &str| dec(native_result(Repr::U(16), j.call(name, &[enc(an, ad), enc(bn, bd)]).unwrap()));
                if ad == 0 || bd == 0 {
                    assert_eq!(call("radd88"), (0, 0), "NaR add {}/{} {}/{}", an, ad, bn, bd);
                    assert_eq!(call("rmul88"), (0, 0), "NaR mul");
                    continue;
                }
                assert_eq!(call("radd88"), fit(an * bd + bn * ad, ad * bd), "add {}/{} {}/{}", an, ad, bn, bd);
                assert_eq!(call("rsub88"), fit(an * bd - bn * ad, ad * bd), "sub {}/{} {}/{}", an, ad, bn, bd);
                assert_eq!(call("rmul88"), fit(an * bn, ad * bd), "mul {}/{} {}/{}", an, ad, bn, bd);
                let want_div = if bn == 0 { (0, 0) } else { fit(an * bd * bn.signum(), ad * bn.abs()) };
                assert_eq!(call("rdiv88"), want_div, "div {}/{} {}/{}", an, ad, bn, bd);
                let lt = native_result(Repr::U(1), j.call("rlt88", &[enc(an, ad), enc(bn, bd)]).unwrap());
                assert_eq!(lt, (an * bd < bn * ad) as i64, "lt {}/{} {}/{}", an, ad, bn, bd);
            }
        }
    }
}
