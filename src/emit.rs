//! The emitter: SSA -> machine code, using ONLY the learned encoding table.
//!
//! No instruction encodings appear in this file — every 32-bit word is
//! produced by `Encoder::encode(template, operands)` against the JSON the
//! prober learned and verified. What *does* live here is strategy:
//! instruction selection (which templates realize each SSA op), the frame
//! layout, and branch/call fixups.
//!
//! Register strategy (v0, correct-not-fast): every SSA value gets an 8-byte
//! stack slot. Each instruction loads its operands into scratch registers
//! (x9/x10/x11), computes, and stores the result back. Nothing lives in a
//! register across an instruction, so calls clobber nothing that matters.
//!
//! Frame layout (sp stays put for the whole body):
//!     sp + 0        saved x29, x30
//!     sp + 16+8*i   slot for value i
//!
//! 32-bit (i32) ops use the w-register templates; w-ops zero the high half,
//! so slots always hold values zero-extended to 64 bits and are always
//! loaded/stored as x registers. i1 values are 0/1 in a full slot.

use crate::ssa::{BinOp, BlockId, CastOp, Cond, Function, Inst, Module, Type, ValueId};
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
}

// templates, named once (the strings must match the seed file exactly)
const LDR_SP: &str = "ldr {x}, [sp, #{i 0..32760 /8}]";
const STR_SP: &str = "str {x}, [sp, #{i 0..32760 /8}]";
const CSET: &str = "cset {x}, {e eq|ne|lt|le|gt|ge|lo|ls|hi|hs}";

fn cond_name(c: Cond) -> &'static str {
    match c {
        Cond::Eq => "eq",
        Cond::Ne => "ne",
        Cond::Slt => "lt",
        Cond::Sle => "le",
        Cond::Sgt => "gt",
        Cond::Sge => "ge",
        Cond::Ult => "lo",
        Cond::Ule => "ls",
        Cond::Ugt => "hi",
        Cond::Uge => "hs",
    }
}

enum FixTarget {
    Block(BlockId),
    Func(String),
}

struct Fixup {
    at: usize, // byte offset of the instruction in `code`
    template: &'static str,
    values: Vec<i64>, // offset slot holds a placeholder, patched later
    imm_slot: usize,
    target: FixTarget,
}

pub fn compile(module: &Module, enc: &Encoder) -> Result<Compiled, String> {
    let mut code: Vec<u8> = Vec::new();
    let mut funcs = HashMap::new();
    let mut call_fixups: Vec<Fixup> = Vec::new();

    for func in &module.funcs {
        funcs.insert(func.name.clone(), code.len());
        compile_function(func, enc, &mut code, &mut call_fixups)
            .map_err(|e| format!("@{}: {}", func.name, e))?;
    }

    // cross-function fixups (bl)
    for fix in call_fixups {
        let target = *funcs
            .get(match &fix.target {
                FixTarget::Func(name) => name.as_str(),
                _ => unreachable!(),
            })
            .ok_or_else(|| {
                let FixTarget::Func(name) = &fix.target else {
                    unreachable!()
                };
                format!("call to undefined function @{}", name)
            })?;
        let mut values = fix.values;
        values[fix.imm_slot] = target as i64 - fix.at as i64;
        let word = enc.encode(fix.template, &values)?;
        code[fix.at..fix.at + 4].copy_from_slice(&word.to_le_bytes());
    }

    Ok(Compiled { code, funcs })
}

struct FnEmit<'a> {
    enc: &'a Encoder,
    func: &'a Function,
    code: &'a mut Vec<u8>,
    frame: i64,
    block_offsets: Vec<Option<usize>>,
    fixups: Vec<Fixup>,
}

impl FnEmit<'_> {
    fn emit(&mut self, template: &'static str, values: &[i64]) -> Result<usize, String> {
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

    fn slot(&self, v: ValueId) -> i64 {
        16 + 8 * v.0 as i64
    }

    /// slot -> register (always a full 64-bit load; slots are zero-extended)
    fn load(&mut self, reg: i64, v: ValueId) -> Result<(), String> {
        let off = self.slot(v);
        self.emit(LDR_SP, &[reg, off]).map(|_| ())
    }

    fn store(&mut self, reg: i64, v: ValueId) -> Result<(), String> {
        let off = self.slot(v);
        self.emit(STR_SP, &[reg, off]).map(|_| ())
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

    /// move branch arguments into the target block's parameter slots.
    /// Loads all args into x9..x16 first, then stores — so swaps like
    /// jmp ^loop(%b, %a) into ^loop(%a, %b) can't clobber each other.
    fn branch_args(&mut self, target: BlockId, args: &[ValueId]) -> Result<(), String> {
        if args.len() > 8 {
            return Err("more than 8 branch arguments not supported yet".into());
        }
        for (j, &a) in args.iter().enumerate() {
            self.load(9 + j as i64, a)?;
        }
        let params: Vec<ValueId> = self.func.blocks[target.0 as usize].params.clone();
        for (j, &p) in params.iter().enumerate() {
            self.store(9 + j as i64, p)?;
        }
        Ok(())
    }

    fn epilogue(&mut self) -> Result<(), String> {
        self.emit("ldp {x}, {x}, [sp, #{i -512..504 /8}]", &[29, 30, 0])?;
        self.emit("add sp, sp, #{i 0..4095}", &[self.frame])?;
        self.emit("ret", &[])?;
        Ok(())
    }
}

/// pick the x or w form of a template by operand type
macro_rules! xw {
    ($ty:expr, $x:expr, $w:expr) => {
        if $ty == Type::I32 {
            $w
        } else {
            $x
        }
    };
}

fn compile_function(
    func: &Function,
    enc: &Encoder,
    code: &mut Vec<u8>,
    call_fixups: &mut Vec<Fixup>,
) -> Result<(), String> {
    let nslots = func.values.len() as i64;
    let frame = (16 + 8 * nslots + 15) & !15;
    if frame > 4095 {
        return Err("function needs too large a frame for v0".into());
    }
    if func.params.len() > 8 {
        return Err("more than 8 parameters not supported yet".into());
    }

    let mut e = FnEmit {
        enc,
        func,
        code,
        frame,
        block_offsets: vec![None; func.blocks.len()],
        fixups: Vec::new(),
    };

    // prologue
    e.emit("sub sp, sp, #{i 0..4095}", &[frame])?;
    e.emit("stp {x}, {x}, [sp, #{i -512..504 /8}]", &[29, 30, 0])?;
    e.emit("mov x29, sp", &[])?;
    for (i, &p) in func.params.iter().enumerate() {
        e.store(i as i64, p)?;
    }

    for (bi, block) in func.blocks.iter().enumerate() {
        e.block_offsets[bi] = Some(e.code.len());
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
            FixTarget::Func(_) => call_fixups.push(fix),
        }
    }
    Ok(())
}

fn compile_inst(e: &mut FnEmit, inst: &Inst) -> Result<(), String> {
    match inst {
        Inst::IConst { dst, imm } => {
            let v = *imm as u64;
            let chunks: Vec<(i64, u16)> = (0..4).map(|i| (i, (v >> (16 * i)) as u16)).collect();
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
            for &(i, c) in &chunks {
                if c == 0 {
                    continue;
                }
                let t = if first { movz[i as usize] } else { movk[i as usize] };
                e.emit(t, &[9, c as i64])?;
                first = false;
            }
            if first {
                e.emit(movz[0], &[9, 0])?; // the constant 0
            }
            e.store(9, *dst)
        }
        Inst::Bin { op, dst, lhs, rhs } => {
            let ty = e.func.ty(*dst);
            e.load(9, *lhs)?;
            e.load(10, *rhs)?;
            let simple: Option<&'static str> = match op {
                BinOp::IAdd => Some(xw!(ty, "add {x}, {x}, {x}", "add {w}, {w}, {w}")),
                BinOp::ISub => Some(xw!(ty, "sub {x}, {x}, {x}", "sub {w}, {w}, {w}")),
                BinOp::IMul => Some(xw!(ty, "mul {x}, {x}, {x}", "mul {w}, {w}, {w}")),
                BinOp::SDiv => Some(xw!(ty, "sdiv {x}, {x}, {x}", "sdiv {w}, {w}, {w}")),
                BinOp::UDiv => Some(xw!(ty, "udiv {x}, {x}, {x}", "udiv {w}, {w}, {w}")),
                BinOp::And => Some(xw!(ty, "and {x}, {x}, {x}", "and {w}, {w}, {w}")),
                BinOp::Or => Some(xw!(ty, "orr {x}, {x}, {x}", "orr {w}, {w}, {w}")),
                BinOp::Xor => Some(xw!(ty, "eor {x}, {x}, {x}", "eor {w}, {w}, {w}")),
                BinOp::Shl => Some(xw!(ty, "lsl {x}, {x}, {x}", "lsl {w}, {w}, {w}")),
                BinOp::LShr => Some(xw!(ty, "lsr {x}, {x}, {x}", "lsr {w}, {w}, {w}")),
                BinOp::AShr => Some(xw!(ty, "asr {x}, {x}, {x}", "asr {w}, {w}, {w}")),
                BinOp::SRem | BinOp::URem => None,
            };
            match simple {
                Some(t) => {
                    e.emit(t, &[9, 9, 10])?;
                }
                None => {
                    // r = a - (a div b) * b
                    let div = match op {
                        BinOp::SRem => xw!(ty, "sdiv {x}, {x}, {x}", "sdiv {w}, {w}, {w}"),
                        _ => xw!(ty, "udiv {x}, {x}, {x}", "udiv {w}, {w}, {w}"),
                    };
                    e.emit(div, &[11, 9, 10])?;
                    e.emit(
                        xw!(ty, "msub {x}, {x}, {x}, {x}", "msub {w}, {w}, {w}, {w}"),
                        &[9, 11, 10, 9],
                    )?;
                }
            }
            e.store(9, *dst)
        }
        Inst::ICmp {
            cond,
            dst,
            lhs,
            rhs,
        } => {
            let ty = e.func.ty(*lhs);
            e.load(9, *lhs)?;
            e.load(10, *rhs)?;
            e.emit(xw!(ty, "cmp {x}, {x}", "cmp {w}, {w}"), &[9, 10])?;
            let ci = e.enc.enum_index(CSET, cond_name(*cond))?;
            e.emit(CSET, &[9, ci])?;
            e.store(9, *dst)
        }
        Inst::Cast { op, dst, src } => {
            let from = e.func.ty(*src);
            let to = e.func.ty(*dst);
            e.load(9, *src)?;
            match (op, from, to) {
                (CastOp::Sext, Type::I1, Type::I64) => {
                    e.emit("sbfx {x}, {x}, #0, #1", &[9, 9])?;
                }
                (CastOp::Sext, Type::I1, Type::I32) => {
                    e.emit("sbfx {w}, {w}, #0, #1", &[9, 9])?;
                }
                (CastOp::Sext, Type::I32, Type::I64) => {
                    e.emit("sxtw {x}, {w}", &[9, 9])?;
                }
                (CastOp::Zext, Type::I1, _) => {
                    e.emit("and {x}, {x}, #1", &[9, 9])?;
                }
                (CastOp::Zext, Type::I32, Type::I64) => {
                    e.emit("mov {w}, {w}", &[9, 9])?; // clears the high half
                }
                (CastOp::Trunc, _, Type::I32) => {
                    e.emit("mov {w}, {w}", &[9, 9])?;
                }
                (CastOp::Trunc, _, Type::I1) => {
                    e.emit("and {x}, {x}, #1", &[9, 9])?;
                }
                _ => return Err(format!("unsupported cast {:?} -> {:?}", from, to)),
            }
            e.store(9, *dst)
        }
        Inst::Load { dst, addr } => {
            let ty = e.func.ty(*dst);
            e.load(9, *addr)?;
            e.emit(
                xw!(
                    ty,
                    "ldr {x}, [{x}, #{i 0..32760 /8}]",
                    "ldr {w}, [{x}, #{i 0..16380 /4}]"
                ),
                &[9, 9, 0],
            )?;
            e.store(9, *dst)
        }
        Inst::Store { val, addr } => {
            let ty = e.func.ty(*val);
            e.load(10, *val)?;
            e.load(9, *addr)?;
            e.emit(
                xw!(
                    ty,
                    "str {x}, [{x}, #{i 0..32760 /8}]",
                    "str {w}, [{x}, #{i 0..16380 /4}]"
                ),
                &[10, 9, 0],
            )
            .map(|_| ())
        }
        Inst::PtrAdd { dst, base, off } => {
            e.load(9, *base)?;
            e.load(10, *off)?;
            e.emit("add {x}, {x}, {x}", &[9, 9, 10])?;
            e.store(9, *dst)
        }
        Inst::Call { dsts, callee, args } => {
            if args.len() > 8 {
                return Err("more than 8 call arguments not supported yet".into());
            }
            for (j, &a) in args.iter().enumerate() {
                e.load(j as i64, a)?;
            }
            let at = e.emit("bl #{i -134217728..134217724 /4}", &[0])?;
            e.fixups.push(Fixup {
                at,
                template: "bl #{i -134217728..134217724 /4}",
                values: vec![0],
                imm_slot: 0,
                target: FixTarget::Func(callee.clone()),
            });
            // results arrive in x0..x7, mirroring how arguments went out
            for (j, &d) in dsts.iter().enumerate() {
                e.store(j as i64, d)?;
            }
            Ok(())
        }
        Inst::Jmp { target, args } => {
            e.branch_args(*target, args)?;
            e.branch("b #{i -134217728..134217724 /4}", vec![0], 0, *target)
        }
        Inst::Br {
            cond,
            then_target,
            then_args,
            else_target,
            else_args,
        } => {
            e.load(9, *cond)?;
            // cbz x9 -> (else path, emitted after the then path); patched below
            let cbz_at = e.emit("cbz {x}, #{i -1048576..1048572 /4}", &[9, 0])?;
            e.branch_args(*then_target, then_args)?;
            e.branch("b #{i -134217728..134217724 /4}", vec![0], 0, *then_target)?;
            let else_here = e.code.len() as i64 - cbz_at as i64;
            e.patch(
                cbz_at,
                "cbz {x}, #{i -1048576..1048572 /4}",
                &[9, else_here],
            )?;
            e.branch_args(*else_target, else_args)?;
            e.branch("b #{i -134217728..134217724 /4}", vec![0], 0, *else_target)
        }
        Inst::Ret { vals } => {
            if vals.len() > 8 {
                return Err("more than 8 return values not supported yet".into());
            }
            for (j, &v) in vals.iter().enumerate() {
                e.load(j as i64, v)?;
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

    pub struct JitCode {
        base: *mut u8,
        #[allow(dead_code)]
        len: usize,
        funcs: std::collections::HashMap<String, usize>,
    }

    impl JitCode {
        pub fn new(compiled: &Compiled) -> Result<JitCode, String> {
            let len = compiled.code.len().next_multiple_of(16384);
            unsafe {
                let base = mmap(
                    std::ptr::null_mut(),
                    len,
                    PROT_READ | PROT_WRITE | PROT_EXEC,
                    MAP_PRIVATE | MAP_ANON | MAP_JIT,
                    -1,
                    0,
                );
                if base as isize == -1 {
                    return Err("mmap(MAP_JIT) failed".into());
                }
                pthread_jit_write_protect_np(0);
                std::ptr::copy_nonoverlapping(compiled.code.as_ptr(), base, compiled.code.len());
                pthread_jit_write_protect_np(1);
                sys_icache_invalidate(base, len);
                Ok(JitCode {
                    base,
                    len,
                    funcs: compiled.funcs.clone(),
                })
            }
        }

        /// Call a compiled function with up to 6 integer arguments.
        pub fn call(&self, name: &str, args: &[i64]) -> Result<i64, String> {
            let &off = self
                .funcs
                .get(name)
                .ok_or_else(|| format!("no function @{} in module", name))?;
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
                n => return Err(format!("{} arguments not supported", n)),
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
                .ok_or_else(|| format!("no function @{} in module", name))?;
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
                n => return Err(format!("{} arguments not supported", n)),
            };
            Ok((r.0, r.1))
        }
    }
}

// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn jit(src: &str) -> jit::JitCode {
        let module = crate::ssa::parse(src).expect("parse");
        crate::ssa::verify(&module).expect("verify");
        let enc = Encoder::load("targets/arm64.encodings.json").expect("encodings");
        let compiled = compile(&module, &enc).expect("compile");
        jit::JitCode::new(&compiled).expect("jit map")
    }

    #[test]
    fn i32_arithmetic_wraps() {
        let j = jit(r"
fn @addmul(%a: i32, %b: i32) -> i32 {
^entry:
    %s: i32 = iadd %a, %b
    %p: i32 = imul %s, %b
    ret %p
}
");
        assert_eq!(j.call("addmul", &[3, 4]).unwrap(), 28);
        // i32 semantics: results wrap at 32 bits and stay zero-extended
        assert_eq!(
            j.call("addmul", &[0x7fffffff, 1]).unwrap(),
            ((0x80000000u64.wrapping_mul(1)) & 0xffffffff) as i64
        );
    }

    #[test]
    fn casts_of_negatives() {
        let j = jit(r"
fn @half_sext(%a: i32) -> i64 {
^entry:
    %w: i64 = sext %a
    %two: i64 = iconst 2
    %h: i64 = sdiv %w, %two
    ret %h
}
");
        // %a arrives zero-extended; sext must recover the sign from bit 31
        assert_eq!(j.call("half_sext", &[0xfffffff6]).unwrap(), -5); // -10 / 2
        assert_eq!(j.call("half_sext", &[10]).unwrap(), 5);
    }

    #[test]
    fn i1_sign_extension() {
        let j = jit(r"
fn @mask(%a: i64, %b: i64) -> i64 {
^entry:
    %lt: i1 = icmp.slt %a, %b
    %m: i64 = sext %lt
    ret %m
}
");
        assert_eq!(j.call("mask", &[1, 2]).unwrap(), -1);
        assert_eq!(j.call("mask", &[2, 1]).unwrap(), 0);
    }

    #[test]
    fn memory_swap() {
        let j = jit(r"
fn @swap(%p: ptr, %q: ptr) {
^entry:
    %a: i64 = load %p
    %b: i64 = load %q
    store %b, %p
    store %a, %q
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
fn @sum4(%p: ptr) -> i32 {
^entry:
    %zero32: i32 = iconst 0
    %zero: i64 = iconst 0
    jmp ^loop(%zero, %zero32)
^loop(%i: i64, %acc: i32):
    %four: i64 = iconst 4
    %done: i1 = icmp.sge %i, %four
    br %done, ^exit, ^body
^body:
    %off: i64 = imul %i, %four
    %q: ptr = ptradd %p, %off
    %v: i32 = load %q
    %acc2: i32 = iadd %acc, %v
    %one: i64 = iconst 1
    %i2: i64 = iadd %i, %one
    jmp ^loop(%i2, %acc2)
^exit:
    ret %acc
}
");
        let data: [i32; 4] = [10, 20, 30, 40];
        assert_eq!(j.call("sum4", &[data.as_ptr() as i64]).unwrap(), 100);
    }

    #[test]
    fn shifts_rems_unsigned() {
        let j = jit(r"
fn @mix(%a: i64, %b: i64) -> i64 {
^entry:
    %sh: i64 = shl %a, %b
    %r: i64 = urem %sh, %a
    %x: i64 = xor %r, %b
    ret %x
}
");
        // (7 << 3) = 56; 56 % 7 = 0; 0 ^ 3 = 3
        assert_eq!(j.call("mix", &[7, 3]).unwrap(), 3);
    }

    #[test]
    fn negative_iconst() {
        let j = jit(r"
fn @neg() -> i64 {
^entry:
    %m: i64 = iconst -42
    ret %m
}
");
        assert_eq!(j.call("neg", &[]).unwrap(), -42);
    }
}
