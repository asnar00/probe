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

use crate::platform::{FOp, Native, Platform};
use crate::ssa::{BinOp, BlockId, Cond, Function, Inst, Module, Repr, ValueId};
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
    let natives = platform.natives(module);
    let mut code: Vec<u8> = Vec::new();
    let mut funcs = HashMap::new();
    let mut call_fixups: Vec<Fixup> = Vec::new();

    for func in &module.funcs {
        funcs.insert(func.name.clone(), code.len());
        compile_function(func, enc, &natives, &mut code, &mut call_fixups)
            .map_err(|e| format!("{}: {}", func.name, e))?;
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
                format!("call to undefined function {}", name)
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
    natives: &'a HashMap<String, Native>,
    code: &'a mut Vec<u8>,
    frame: i64,
    alloc: &'a crate::regalloc::Alloc,
    spill_base: i64,
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

    fn slot_off(&self, idx: usize) -> i64 {
        self.spill_base + 8 * idx as i64
    }

    /// register currently holding v: its allocated register, or the spill
    /// slot loaded into `scratch`
    fn src_reg(&mut self, v: ValueId, scratch: i64) -> Result<i64, String> {
        match self.alloc.loc[v.0 as usize] {
            crate::regalloc::Loc::Reg(r) => Ok(r),
            crate::regalloc::Loc::Slot(i) => {
                let off = self.slot_off(i);
                self.emit(LDR_SP, &[scratch, off])?;
                Ok(scratch)
            }
        }
    }

    /// register a result should be computed into
    fn dst_reg(&self, v: ValueId, scratch: i64) -> i64 {
        match self.alloc.loc[v.0 as usize] {
            crate::regalloc::Loc::Reg(r) => r,
            crate::regalloc::Loc::Slot(_) => scratch,
        }
    }

    /// after computing into dst_reg(v): spill if v lives on the stack
    fn finish(&mut self, v: ValueId, reg: i64) -> Result<(), String> {
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
        match self.alloc.loc[v.0 as usize] {
            crate::regalloc::Loc::Reg(r) => self.mov(r, source),
            crate::regalloc::Loc::Slot(i) => {
                let off = self.slot_off(i);
                self.emit(STR_SP, &[source, off]).map(|_| ())
            }
        }
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

    /// one location-to-location move (registers or spill slots)
    fn loc_move(
        &mut self,
        dst: crate::regalloc::Loc,
        src: crate::regalloc::Loc,
    ) -> Result<(), String> {
        use crate::regalloc::Loc;
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
        let mut pending: Vec<(Loc, Loc)> = params
            .iter()
            .zip(args)
            .map(|(&p, &a)| (self.alloc.loc[p.0 as usize], self.alloc.loc[a.0 as usize]))
            .filter(|(d, s)| d != s)
            .collect();
        let scratch = Loc::Reg(9);
        while !pending.is_empty() {
            if let Some(i) = (0..pending.len())
                .find(|&i| !pending.iter().any(|&(_, s)| s == pending[i].0))
            {
                let (d, s) = pending.swap_remove(i);
                self.loc_move(d, s)?;
            } else {
                // pure cycle: stash one source in the scratch register
                let s = pending[0].1;
                self.loc_move(scratch, s)?;
                for m in pending.iter_mut().filter(|m| m.1 == s) {
                    m.1 = scratch;
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

    fn epilogue(&mut self) -> Result<(), String> {
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
        self.emit("ldp {x}, {x}, [sp, #{i -512..504 /8}]", &[29, 30, 0])?;
        self.emit("add sp, sp, #{i 0..4095}", &[self.frame])?;
        self.emit("ret", &[])?;
        Ok(())
    }
}

/// pool for the allocator: callee-saved x19..x28 — values placed here
/// survive calls by construction, so call sites need no spill logic
const REG_POOL: &[i64] = &[19, 20, 21, 22, 23, 24, 25, 26, 27, 28];

/// Compile one function into a standalone buffer that will live at arena
/// offset `base`; calls resolve through `resolve` (name -> arena offset of
/// the callee's entry — in the incremental arena, its trampoline).
pub fn compile_one(
    func: &Function,
    enc: &Encoder,
    natives: &HashMap<String, Native>,
    base: i64,
    resolve: &dyn Fn(&str) -> Option<i64>,
) -> Result<Vec<u8>, String> {
    let mut code = Vec::new();
    let mut fixups = Vec::new();
    compile_function(func, enc, natives, &mut code, &mut fixups)
        .map_err(|e| format!("{}: {}", func.name, e))?;
    for fix in fixups {
        let FixTarget::Func(name) = &fix.target else {
            unreachable!()
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
    natives: &HashMap<String, Native>,
    code: &mut Vec<u8>,
    call_fixups: &mut Vec<Fixup>,
) -> Result<(), String> {
    if let Some(&op) = natives.get(&func.name) {
        // this function *is* a platform instruction: x0, x1 -> x0, no frame
        native_body(enc, op, code)?;
        return Ok(());
    }
    let alloc = crate::regalloc::allocate(func, REG_POOL);
    let nsaved = alloc.used_regs.len() as i64;
    let spill_base = 16 + 8 * nsaved;
    let frame = (spill_base + 8 * alloc.nslots as i64 + 15) & !15;
    if frame > 4095 {
        return Err("function needs too large a frame for v0".into());
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
        spill_base,
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
    for (i, &p) in func.params.iter().enumerate() {
        e.value_from(p, i as i64)?;
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

/// (move in, the op, move out) for a platform float op
fn fp_templates(op: Native) -> (&'static str, &'static str, &'static str) {
    match (op.op, op.bits) {
        (FOp::Add, 32) => ("fmov {s}, {w}", "fadd {s}, {s}, {s}", "fmov {w}, {s}"),
        (FOp::Sub, 32) => ("fmov {s}, {w}", "fsub {s}, {s}, {s}", "fmov {w}, {s}"),
        (FOp::Mul, 32) => ("fmov {s}, {w}", "fmul {s}, {s}, {s}", "fmov {w}, {s}"),
        (FOp::Div, 32) => ("fmov {s}, {w}", "fdiv {s}, {s}, {s}", "fmov {w}, {s}"),
        (FOp::Add, _) => ("fmov {d}, {x}", "fadd {d}, {d}, {d}", "fmov {x}, {d}"),
        (FOp::Sub, _) => ("fmov {d}, {x}", "fsub {d}, {d}, {d}", "fmov {x}, {d}"),
        (FOp::Mul, _) => ("fmov {d}, {x}", "fmul {d}, {d}, {d}", "fmov {x}, {d}"),
        (FOp::Div, _) => ("fmov {d}, {x}", "fdiv {d}, {d}, {d}", "fmov {x}, {d}"),
    }
}

/// the whole body of a natively implemented function: the sequence on the
/// argument registers, result in x0, and return
fn native_body(enc: &Encoder, op: Native, code: &mut Vec<u8>) -> Result<(), String> {
    let (to_f, fop, to_i) = fp_templates(op);
    let seq: [(&str, &[i64]); 5] = [
        (to_f, &[16, 0]),
        (to_f, &[17, 1]),
        (fop, &[16, 16, 17]),
        (to_i, &[0, 16]),
        ("ret", &[]),
    ];
    for (t, v) in seq {
        code.extend_from_slice(&enc.encode(t, v)?.to_le_bytes());
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
            let mut v = crate::opt::norm(r, *imm) as u64;
            if r.container() == 32 {
                v &= 0xffff_ffff;
            }
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
                e.emit(t, &[rd, c as i64])?;
                first = false;
            }
            if first {
                e.emit(movz[0], &[rd, 0])?; // the constant 0
            }
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
        Inst::Load { dst, addr } => {
            let r = e.repr(*dst);
            let ra = e.src_reg(*addr, 9)?;
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
            e.emit(t, &[rd, ra, 0])?;
            e.finish(*dst, rd)
        }
        Inst::Store { val, addr } => {
            let r = e.repr(*val);
            let rv = e.src_reg(*val, 10)?;
            let ra = e.src_reg(*addr, 9)?;
            let t = match r.bits() {
                8 => "strb {w}, [{x}, #{i 0..4095}]",
                16 => "strh {w}, [{x}, #{i 0..8190 /2}]",
                32 => "str {w}, [{x}, #{i 0..16380 /4}]",
                64 => "str {x}, [{x}, #{i 0..32760 /8}]",
                n => return Err(format!("no {}-bit memory access", n)),
            };
            e.emit(t, &[rv, ra, 0]).map(|_| ())
        }
        Inst::PtrAdd { dst, base, off } => {
            let rb = e.src_reg(*base, 9)?;
            let ro = e.src_reg(*off, 10)?;
            let rd = e.dst_reg(*dst, 9);
            e.emit("add {x}, {x}, {x}", &[rd, rb, ro])?;
            e.finish(*dst, rd)
        }
        Inst::Call { dsts, callee, args } if e.natives.contains_key(callee) => {
            // the platform has this one: the instruction instead of the call
            let op = e.natives[callee];
            let Some(&dst) = dsts.first() else {
                return Ok(()); // result unused: nothing to compute
            };
            let rl = e.src_reg(args[0], 9)?;
            let rr = e.src_reg(args[1], 10)?;
            let rd = e.dst_reg(dst, 9);
            // through v16/v17 (caller-saved, never allocated)
            let (to_f, fop, to_i) = fp_templates(op);
            e.emit(to_f, &[16, rl])?;
            e.emit(to_f, &[17, rr])?;
            e.emit(fop, &[16, 16, 17])?;
            e.emit(to_i, &[rd, 16])?;
            e.finish(dst, rd)
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
            e.branch("b #{i -134217728..134217724 /4}", vec![0], 0, *target)
        }
        Inst::Br {
            cond,
            then_target,
            then_args,
            else_target,
            else_args,
        } => {
            let rc = e.src_reg(*cond, 9)?;
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
            e.branch("b #{i -134217728..134217724 /4}", vec![0], 0, *else_target)
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
    w: i64 = ext a
    two: i64 = iconst 2
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
    lt: u1 = icmp.lt a, b
    s: i1 = bitcast lt
    m: i64 = ext s
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
    zero32: i32 = iconst 0
    zero: i64 = iconst 0
    jmp loop(zero, zero32)
loop(i: i64, acc: i32):
    four: i64 = iconst 4
    done: u1 = icmp.ge i, four
    br done, exit, body
body:
    off: i64 = mul i, four
    q: ptr = ptradd p, off
    v: i32 = load q
    acc2: i32 = add acc, v
    one: i64 = iconst 1
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
    m: i64 = iconst -42
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
                    "fn c_{n}(a: {t}, b: {t}) -> u1 {{\nentry:\n    r: u1 = icmp.{n} a, b\n    ret r\n}}\n",
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
                        assert_eq!(got, want, "{} icmp.{} {} {}", ty, name, a, b);
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
                let op = if to.bits() > from.bits() {
                    "ext"
                } else if to.bits() < from.bits() {
                    "trunc"
                } else {
                    "bitcast"
                };
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
    r6: u64 = ext r
    g6: u64 = ext g
    b6: u64 = ext b
    x: u64 = add r6, g6
    y: u64 = add x, b6
    ret y
}
fn nested(s: i3, w: u16, t: i9, f: u1) -> (i64, i64) {
entry:
    c: rgb = bitcast w
    m: mix = pack s, c, t, f
    s2: i3 = get m, s
    t2: i9 = get m, t
    sw: i64 = ext s2
    tw: i64 = ext t2
    ret sw, tw
}
fn nested_bits(s: i3, w: u16, t: i9, f: u1) -> u64 {
entry:
    c: rgb = bitcast w
    m: mix = pack s, c, t, f
    c2: rgb = get m, c
    cw: u16 = bitcast c2
    f2: u1 = get m, flag
    cw64: u64 = ext cw
    f64: u64 = ext f2
    bits: u29 = bitcast m
    all: u64 = ext bits
    x: u64 = xor all, cw64
    y: u64 = xor x, f64
    ret y
}
fn bytes(p: ptr, v: i8) -> i64 {
entry:
    store v, p
    one: i64 = iconst 1
    q: ptr = ptradd p, one
    u: u8 = bitcast v
    store u, q
    a: i8 = load p
    b: u8 = load q
    aw: i64 = ext a
    bw: i64 = ext b
    r: i64 = sub aw, bw
    ret r
}
fn halves(p: ptr, v: i16) -> i64 {
entry:
    store v, p
    two: i64 = iconst 2
    q: ptr = ptradd p, two
    u: u16 = bitcast v
    store u, q
    a: i16 = load p
    b: u16 = load q
    aw: i64 = ext a
    bw: i64 = ext b
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
        let src = include_str!("../suite/float.ssa");
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
        assert!(size(&hard, "fadd32") < size(&soft, "fadd32") / 4, "{} vs {}", size(&hard, "fadd32"), size(&soft, "fadd32"));
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
        // --- fp8 exhaustive, fp16 / bf16 dense: the exact reference ---
        for (suffix, e, m, n) in [("8", 4u32, 3u32, 0usize), ("16", 5, 10, 200), ("b16", 8, 7, 200)] {
            let w = e + m + 1;
            let vals: Vec<u64> = if n == 0 {
                (0..1u64 << w).collect()
            } else {
                patterns(e, m, n, &mut rnd)
            };
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
        let lib = include_str!("../suite/float.ssa");
        let src = format!(
            "{}\nfn sum32(a: f32, b: f32) -> f32 {{\n    r: f32 = add a, b\n    ret r\n}}\nfn sum16(a: f16, b: f16) -> f16 {{\n    r: f16 = add a, b\n    ret r\n}}\n",
            lib.replace("type f32 = float(8, 23)\n", "type f32 = float(8, 23)\ntype f16 = float(5, 10)\n")
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
        assert_eq!(callee.as_deref(), Some("fadd16"));
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
}
