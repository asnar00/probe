//! Byte-sequence learner for stack-machine targets (WebAssembly).
//!
//! Same method as the register-machine learner — probe an oracle, form a
//! hypothesis, verify it on random values, refuse to output anything that
//! doesn't match 100% — with a different hypothesis family. Wasm
//! instructions are variable-length byte strings: opcode bytes plus LEB128
//! varint immediates. So a learned template here is a sequence of pieces:
//! fixed bytes, and u/s-LEB128 codecs at discovered positions.
//!
//! Probes are wat functions batched into one module; the code section gives
//! each body's bytes exactly (length-prefixed), so no diffing is needed to
//! extract them. Because a probed instruction usually needs stack context
//! (`i64.add` needs two operands), each seed entry declares `pre`/`post`
//! instructions — instantiations of *already-learned* templates, whose
//! bytes we can therefore compute and strip. Learning order in the seed
//! file is the bootstrap chain. The `end` opcode is observed rather than
//! probed: it terminates every function body.
//!
//! Container framing (magic, section headers, body sizes, locals
//! declarations) is parsed with spec knowledge — that's the file format,
//! the analogue of reading .text out of a Mach-O, not the instruction
//! encoding we're learning.

use std::process::Command;

// ---------------------------------------------------------------------------
// Learned representation

#[derive(Clone, Debug, PartialEq)]
pub enum Piece {
    Fixed(Vec<u8>),
    ULeb,
    SLeb,
    /// 8 raw little-endian bytes (f64.const); the slot value is the bits
    Bits64,
    /// 4 raw little-endian bytes (f32.const)
    Bits32,
}

pub fn uleb(mut v: u64) -> Vec<u8> {
    let mut out = Vec::new();
    loop {
        let b = (v & 0x7f) as u8;
        v >>= 7;
        if v == 0 {
            out.push(b);
            return out;
        }
        out.push(b | 0x80);
    }
}

pub fn sleb(mut v: i64) -> Vec<u8> {
    let mut out = Vec::new();
    loop {
        let b = (v & 0x7f) as u8;
        v >>= 7;
        let done = (v == 0 && b & 0x40 == 0) || (v == -1 && b & 0x40 != 0);
        if done {
            out.push(b);
            return out;
        }
        out.push(b | 0x80);
    }
}

pub fn encode_pieces(pieces: &[Piece], value: Option<i64>) -> Vec<u8> {
    let mut out = Vec::new();
    for p in pieces {
        match p {
            Piece::Fixed(b) => out.extend_from_slice(b),
            Piece::ULeb => out.extend(uleb(value.expect("slot value") as u64)),
            Piece::SLeb => out.extend(sleb(value.expect("slot value"))),
            Piece::Bits64 => {
                out.extend((value.expect("slot value") as u64).to_le_bytes())
            }
            Piece::Bits32 => {
                out.extend((value.expect("slot value") as u32).to_le_bytes())
            }
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Seed format

#[derive(Clone, Copy, PartialEq)]
enum WSlot {
    Range(i64, i64),
    F64,
    F32,
}

struct WInst {
    template: String, // e.g. "local.get {i 0..16384}"
    slot: Option<WSlot>,
    sig: String,                    // "(param i64 i64) (result i64)"
    locals: Option<(usize, String)>, // extra locals: (count, type)
    pre: Vec<String>,               // learned-template instantiations
    post: Vec<String>,
    preamble: usize, // dummy "(func)" entries before the probe functions
}

struct WSeed {
    insts: Vec<WInst>,
}

fn parse_wseed(name: &str, src: &str) -> Result<WSeed, String> {
    let mut seed = WSeed { insts: Vec::new() };
    for (ln, raw) in src.lines().enumerate() {
        let line = raw.split(';').next().unwrap().trim();
        if line.is_empty() {
            continue;
        }
        let err = |m: String| format!("{}:{}: {}", name, ln + 1, m);
        let (kw, rest) = line.split_once(char::is_whitespace).unwrap_or((line, ""));
        match kw {
            "target" | "format" | "oracle" => {} // descriptive only
            "inst" => {
                let mut parts = rest.split("::").map(str::trim);
                let template = parts.next().unwrap().to_string();
                let slot = parse_slot(&template).map_err(&err)?;
                let mut inst = WInst {
                    template,
                    slot,
                    sig: String::new(),
                    locals: None,
                    pre: Vec::new(),
                    post: Vec::new(),
                    preamble: 0,
                };
                for part in parts {
                    let (k, v) = part
                        .split_once('=')
                        .ok_or_else(|| err(format!("expected key=value, got '{}'", part)))?;
                    match k.trim() {
                        "sig" => inst.sig = v.trim().to_string(),
                        "locals" => {
                            let (n, ty) = v
                                .trim()
                                .split_once(' ')
                                .ok_or_else(|| err("locals=<count> <type>".into()))?;
                            inst.locals = Some((
                                n.parse().map_err(|_| err("bad locals count".into()))?,
                                ty.trim().to_string(),
                            ));
                        }
                        "pre" => {
                            inst.pre = v.split(',').map(|s| s.trim().to_string()).collect()
                        }
                        "post" => {
                            inst.post = v.split(',').map(|s| s.trim().to_string()).collect()
                        }
                        "preamble" => {
                            inst.preamble =
                                v.trim().parse().map_err(|_| err("bad preamble count".into()))?
                        }
                        k => return Err(err(format!("unknown key '{}'", k))),
                    }
                }
                seed.insts.push(inst);
            }
            _ => return Err(err(format!("unknown directive '{}'", kw))),
        }
    }
    Ok(seed)
}

/// At most one slot per template: `{i lo..hi}`, `{f64}`, or `{f32}`.
fn parse_slot(template: &str) -> Result<Option<WSlot>, String> {
    let Some(open) = template.find('{') else {
        return Ok(None);
    };
    let close = template[open..]
        .find('}')
        .ok_or("unclosed '{'")?
        + open;
    let inner = template[open + 1..close].trim();
    if inner == "f64" {
        return Ok(Some(WSlot::F64));
    }
    if inner == "f32" {
        return Ok(Some(WSlot::F32));
    }
    let spec = inner
        .strip_prefix("i ")
        .ok_or_else(|| format!("bad slot '{{{}}}'", inner))?;
    let dots = spec[1..].find("..").map(|i| i + 1).ok_or("expected lo..hi")?;
    let lo: i64 = spec[..dots].parse().map_err(|_| "bad lo")?;
    let hi: i64 = spec[dots + 2..].parse().map_err(|_| "bad hi")?;
    if template[close + 1..].contains('{') {
        return Err("at most one slot per template".into());
    }
    Ok(Some(WSlot::Range(lo, hi)))
}

/// slot values are integers; float slots carry the BIT PATTERN and render
/// as a shortest-roundtrip decimal literal for the wat text
fn render(template: &str, slot: Option<WSlot>, value: Option<i64>) -> String {
    match (template.find('{'), value) {
        (Some(open), Some(v)) => {
            let close = template[open..].find('}').unwrap() + open;
            let text = match slot {
                Some(WSlot::F64) => format!("{:?}", f64::from_bits(v as u64)),
                Some(WSlot::F32) => format!("{:?}", f32::from_bits(v as u32)),
                _ => v.to_string(),
            };
            format!("{}{}{}", &template[..open], text, &template[close + 1..])
        }
        _ => template.to_string(),
    }
}

// ---------------------------------------------------------------------------
// Oracle: wat2wasm + code-section extraction

fn wat2wasm(scratch: &std::path::Path, wat: &str) -> Result<Vec<u8>, String> {
    let watf = scratch.join("probe.wat");
    let wasmf = scratch.join("probe.wasm");
    std::fs::write(&watf, wat).map_err(|e| e.to_string())?;
    let out = Command::new("wat2wasm")
        .arg(&watf)
        .arg("-o")
        .arg(&wasmf)
        .output()
        .map_err(|e| format!("spawn wat2wasm: {}", e))?;
    if !out.status.success() {
        let msg = String::from_utf8_lossy(&out.stderr);
        return Err(format!(
            "wat2wasm rejected the probe module: {}",
            msg.lines().next().unwrap_or("")
        ));
    }
    std::fs::read(&wasmf).map_err(|e| e.to_string())
}

fn read_uleb(b: &[u8], pos: &mut usize) -> u64 {
    let mut v = 0u64;
    let mut shift = 0;
    loop {
        let byte = b[*pos];
        *pos += 1;
        v |= ((byte & 0x7f) as u64) << shift;
        if byte & 0x80 == 0 {
            return v;
        }
        shift += 7;
    }
}

/// Instruction bytes of each function body (locals declaration and the
/// final `end` stripped; the end byte is returned separately).
fn extract_bodies(wasm: &[u8]) -> Result<(Vec<Vec<u8>>, u8), String> {
    let mut pos = 8; // magic + version
    while pos < wasm.len() {
        let id = wasm[pos];
        pos += 1;
        let size = read_uleb(wasm, &mut pos) as usize;
        if id != 10 {
            pos += size;
            continue;
        }
        let count = read_uleb(wasm, &mut pos) as usize;
        let mut bodies = Vec::with_capacity(count);
        let mut end_byte = 0u8;
        for _ in 0..count {
            let bsize = read_uleb(wasm, &mut pos) as usize;
            let body_end = pos + bsize;
            let ndecls = read_uleb(wasm, &mut pos) as usize;
            for _ in 0..ndecls {
                read_uleb(wasm, &mut pos);
                pos += 1; // valtype
            }
            let instrs = &wasm[pos..body_end];
            if instrs.is_empty() {
                return Err("empty function body".into());
            }
            end_byte = instrs[instrs.len() - 1];
            bodies.push(instrs[..instrs.len() - 1].to_vec());
            pos = body_end;
        }
        return Ok((bodies, end_byte));
    }
    Err("no code section in oracle output".into())
}

// ---------------------------------------------------------------------------
// Learning

pub struct WShapeResult {
    pub text: String,
    pub outcome: Result<(Vec<Piece>, usize), String>, // (pieces, verified) or reason
}

struct Learned {
    templates: Vec<(String, Option<WSlot>, Vec<Piece>)>,
}

impl Learned {
    /// Encode a pre/post item like "local.get 0" against learned templates:
    /// literal words must match, a numeric word binds the slot.
    fn encode_item(&self, item: &str) -> Result<Vec<u8>, String> {
        let iwords: Vec<&str> = item.split_whitespace().collect();
        'templates: for (text, _, pieces) in &self.templates {
            let twords: Vec<&str> = text.split_whitespace().collect();
            // the slot spec "{i lo..hi}" is two words ("{i", "lo..hi}"); rejoin
            let mut tw: Vec<String> = Vec::new();
            let mut i = 0;
            while i < twords.len() {
                if twords[i].starts_with('{') {
                    let mut s = twords[i].to_string();
                    while !s.ends_with('}') {
                        i += 1;
                        s.push(' ');
                        s.push_str(twords[i]);
                    }
                    tw.push(s);
                } else {
                    tw.push(twords[i].to_string());
                }
                i += 1;
            }
            if tw.len() != iwords.len() {
                continue;
            }
            let mut value = None;
            for (t, w) in tw.iter().zip(&iwords) {
                if t.starts_with('{') {
                    match w.parse::<i64>() {
                        Ok(v) => value = Some(v),
                        Err(_) => continue 'templates,
                    }
                } else if t != w {
                    continue 'templates;
                }
            }
            return Ok(encode_pieces(pieces, value));
        }
        Err(format!(
            "pre/post item '{}' does not match any learned template (check seed order)",
            item
        ))
    }

    fn encode_items(&self, items: &[String]) -> Result<Vec<u8>, String> {
        let mut out = Vec::new();
        for it in items {
            out.extend(self.encode_item(it)?);
        }
        Ok(out)
    }
}

/// sample float bit patterns: normal values across signs and magnitudes
/// (no NaN/Inf — tools may canonicalize payloads)
fn float_probe_values(f32bits: bool) -> Vec<i64> {
    let doubles: [f64; 12] = [
        0.0, 1.0, -1.0, 0.5, 2.5, -3.75, 100.0, -0.001, 1e10, -1e-10, 12345.6789, 3.5e38,
    ];
    doubles
        .iter()
        .map(|&d| {
            if f32bits {
                (d as f32).to_bits() as i64
            } else {
                d.to_bits() as i64
            }
        })
        .collect()
}

fn probe_values(lo: i64, hi: i64) -> Vec<i64> {
    let mut vals: Vec<i64> = vec![
        0, 1, 2, 3, 5, 63, 64, 127, 128, 129, 255, 256, 8191, 8192, 16383, 16384,
    ];
    vals.extend([1 << 20, (1 << 21) - 1, (1 << 31) - 1, 1 << 31, 1 << 32, 1 << 62, hi]);
    if lo < 0 {
        vals.extend([
            -1, -2, -63, -64, -65, -127, -128, -129, -8192, -8193, -16384, -16385,
            -(1 << 31), -(1 << 32), -(1 << 62), lo,
        ]);
    }
    vals.retain(|&v| v >= lo && v <= hi);
    vals.sort_unstable();
    vals.dedup();
    vals
}

fn build_module(inst: &WInst, values: &[Option<i64>]) -> String {
    let mut wat = String::from("(module\n  (memory 1)\n");
    for _ in 0..inst.preamble {
        wat.push_str("  (func)\n");
    }
    for &v in values {
        wat.push_str("  (func ");
        wat.push_str(&inst.sig);
        wat.push('\n');
        if let Some((n, ty)) = &inst.locals {
            wat.push_str("    (local");
            for _ in 0..*n {
                wat.push(' ');
                wat.push_str(ty);
            }
            wat.push_str(")\n");
        }
        for p in &inst.pre {
            wat.push_str("    ");
            wat.push_str(p);
            wat.push('\n');
        }
        wat.push_str("    ");
        wat.push_str(&render(&inst.template, inst.slot, v));
        wat.push('\n');
        for p in &inst.post {
            wat.push_str("    ");
            wat.push_str(p);
            wat.push('\n');
        }
        wat.push_str("  )\n");
    }
    wat.push_str(")\n");
    wat
}

/// Strip a known prefix and suffix, or explain what didn't match.
fn strip<'a>(body: &'a [u8], pre: &[u8], post: &[u8]) -> Result<&'a [u8], String> {
    if body.len() < pre.len() + post.len() || !body.starts_with(pre) || !body.ends_with(post) {
        return Err(format!(
            "body {:02x?} does not contain expected pre {:02x?} / post {:02x?}",
            body, pre, post
        ));
    }
    Ok(&body[pre.len()..body.len() - post.len()])
}

/// Fit remainders to `prefix + codec(value) + suffix` with constant
/// prefix/suffix. Tries ULEB then SLEB (SLEB only, if the range is signed).
fn fit_codec(probes: &[(i64, Vec<u8>)], slot: WSlot) -> Result<Vec<Piece>, String> {
    let codecs: &[Piece] = match slot {
        WSlot::F64 => &[Piece::Bits64],
        WSlot::F32 => &[Piece::Bits32],
        WSlot::Range(lo, _) if lo < 0 => &[Piece::SLeb],
        WSlot::Range(..) => &[Piece::ULeb, Piece::SLeb],
    };
    for codec in codecs {
        let enc = |v: i64| match codec {
            Piece::ULeb => uleb(v as u64),
            Piece::SLeb => sleb(v),
            Piece::Bits64 => (v as u64).to_le_bytes().to_vec(),
            Piece::Bits32 => (v as u32).to_le_bytes().to_vec(),
            _ => unreachable!(),
        };
        let max_pl = probes
            .iter()
            .map(|(v, r)| r.len().saturating_sub(enc(*v).len()))
            .min()
            .unwrap_or(0);
        'pl: for pl in 0..=max_pl {
            let prefix = &probes[0].1[..pl];
            let mut suffix: Option<&[u8]> = None;
            for (v, r) in probes {
                let e = enc(*v);
                if r.len() < pl + e.len() || &r[..pl] != prefix || r[pl..pl + e.len()] != e[..] {
                    continue 'pl;
                }
                let s = &r[pl + e.len()..];
                match suffix {
                    None => suffix = Some(s),
                    Some(prev) if prev == s => {}
                    _ => continue 'pl,
                }
            }
            let mut pieces = Vec::new();
            if !prefix.is_empty() {
                pieces.push(Piece::Fixed(prefix.to_vec()));
            }
            pieces.push(codec.clone());
            if let Some(s) = suffix {
                if !s.is_empty() {
                    pieces.push(Piece::Fixed(s.to_vec()));
                }
            }
            return Ok(pieces);
        }
    }
    Err("no prefix + LEB128 + suffix decomposition fits the probes".into())
}

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545F4914F6CDD1D)
    }
    fn range(&mut self, lo: i64, hi: i64) -> i64 {
        lo + (self.next() % ((hi - lo) as u64 + 1)) as i64
    }
}

/// Stratified-by-bit-width verification values, as in the bit learner.
fn verify_values(lo: i64, hi: i64, rng: &mut Rng) -> Vec<i64> {
    let mut vals = vec![lo, hi, 0];
    let mut nbits = 0;
    while 1i64 << nbits < hi && nbits < 62 {
        nbits += 1;
    }
    for _ in 0..24 {
        let w = rng.range(1, nbits.max(1));
        let base = if w == 1 { 1 } else { 1i64 << (w - 1) };
        let mut v = rng.range(base, ((1i64 << w) - 1).min(hi).max(base));
        if lo < 0 && rng.next() & 1 == 1 {
            v = (-v).max(lo);
        }
        vals.push(v.clamp(lo, hi));
    }
    vals.sort_unstable();
    vals.dedup();
    vals
}

pub fn learn_wasm_target(
    name: &str,
    src: &str,
    scratch: &std::path::Path,
) -> Result<(Vec<WShapeResult>, u8), String> {
    let seed = parse_wseed(name, src)?;
    let mut learned = Learned {
        templates: Vec::new(),
    };
    let mut results = Vec::new();
    let mut end_byte: Option<u8> = None;

    for inst in &seed.insts {
        let outcome = learn_one(inst, &learned, scratch, &mut end_byte);
        if let Ok((pieces, _)) = &outcome {
            learned
                .templates
                .push((inst.template.clone(), inst.slot, pieces.clone()));
            // `end` becomes usable in later pre/post as soon as observed
            if let Some(e) = end_byte {
                if !learned.templates.iter().any(|(t, _, _)| t == "end") {
                    learned
                        .templates
                        .push(("end".into(), None, vec![Piece::Fixed(vec![e])]));
                }
            }
        }
        results.push(WShapeResult {
            text: inst.template.clone(),
            outcome,
        });
    }
    let end = end_byte.ok_or("no probe succeeded; end opcode never observed")?;
    Ok((results, end))
}

fn learn_one(
    inst: &WInst,
    learned: &Learned,
    scratch: &std::path::Path,
    end_byte: &mut Option<u8>,
) -> Result<(Vec<Piece>, usize), String> {
    let pre = learned.encode_items(&inst.pre)?;
    let post = learned.encode_items(&inst.post)?;

    let values: Vec<Option<i64>> = match inst.slot {
        Some(WSlot::Range(lo, hi)) => probe_values(lo, hi).into_iter().map(Some).collect(),
        Some(WSlot::F64) => float_probe_values(false).into_iter().map(Some).collect(),
        Some(WSlot::F32) => float_probe_values(true).into_iter().map(Some).collect(),
        None => vec![None],
    };
    let wat = build_module(inst, &values);
    let wasm = wat2wasm(scratch, &wat)?;
    let (bodies, end) = extract_bodies(&wasm)?;
    if let Some(prev) = *end_byte {
        if prev != end {
            return Err("inconsistent end opcode across probes".into());
        }
    }
    *end_byte = Some(end);
    let bodies = &bodies[inst.preamble..];
    if bodies.len() != values.len() {
        return Err("oracle returned a different number of functions".into());
    }

    let mut remainders = Vec::new();
    for (v, body) in values.iter().zip(bodies) {
        remainders.push((v.unwrap_or(0), strip(body, &pre, &post)?.to_vec()));
    }

    // an instruction that encodes to nothing means the oracle optimized the
    // probe away (e.g. wat2wasm elides an empty else arm) — never accept it
    if remainders.iter().any(|(_, r)| r.is_empty()) {
        return Err("instruction encodes to zero bytes (oracle optimized the probe away)".into());
    }
    let pieces = match inst.slot {
        None => vec![Piece::Fixed(remainders[0].1.clone())],
        Some(slot) => {
            for (_, r) in &remainders[1..] {
                if *r == remainders[0].1 {
                    return Err("slot value does not affect the encoding".into());
                }
            }
            fit_codec(&remainders, slot)?
        }
    };

    // ---- verify on fresh values ----
    let tested = match inst.slot {
        None => 1, // nothing further to vary; the single probe is the proof
        Some(slot) => {
            let mut rng = Rng(0xA076_1D64_78BD_642F);
            let raw: Vec<i64> = match slot {
                WSlot::Range(lo, hi) => verify_values(lo, hi, &mut rng),
                // random normal floats: bounded exponent, random mantissa
                WSlot::F64 => (0..24)
                    .map(|_| {
                        let sign = (rng.next() & 1) << 63;
                        let exp = (896 + (rng.next() % 256)) << 52; // 2^-127..2^128
                        let man = rng.next() & ((1 << 52) - 1);
                        (sign | exp | man) as i64
                    })
                    .collect(),
                WSlot::F32 => (0..24)
                    .map(|_| {
                        let sign = ((rng.next() & 1) as u32) << 31;
                        let exp = ((64 + (rng.next() % 128)) as u32) << 23;
                        let man = (rng.next() as u32) & ((1 << 23) - 1);
                        (sign | exp | man) as i64
                    })
                    .collect(),
            };
            let vvals: Vec<Option<i64>> = raw.into_iter().map(Some).collect();
            let wat = build_module(inst, &vvals);
            let wasm = wat2wasm(scratch, &wat)?;
            let (bodies, _) = extract_bodies(&wasm)?;
            let bodies = &bodies[inst.preamble..];
            for (v, body) in vvals.iter().zip(bodies) {
                let inst_bytes = strip(body, &pre, &post)?;
                let predicted = encode_pieces(&pieces, *v);
                if predicted != inst_bytes {
                    return Err(format!(
                        "verification mismatch at value {}: predicted {:02x?}, oracle {:02x?}",
                        v.unwrap(),
                        predicted,
                        inst_bytes
                    ));
                }
            }
            vvals.len()
        }
    };

    Ok((pieces, tested))
}

// ---------------------------------------------------------------------------
// Reporting and serialization

pub fn wreport(results: &[WShapeResult]) -> String {
    let mut out = String::new();
    let ok = results.iter().filter(|r| r.outcome.is_ok()).count();
    for r in results {
        match &r.outcome {
            Ok((pieces, tested)) => {
                let ps: Vec<String> = pieces
                    .iter()
                    .map(|p| match p {
                        Piece::Fixed(b) => b
                            .iter()
                            .map(|x| format!("{:02x}", x))
                            .collect::<Vec<_>>()
                            .join(""),
                        Piece::ULeb => "uleb".into(),
                        Piece::SLeb => "sleb".into(),
                        Piece::Bits64 => "bits64".into(),
                        Piece::Bits32 => "bits32".into(),
                    })
                    .collect();
                out.push_str(&format!(
                    "  ok  {:<34} {}  (verified {})\n",
                    r.text,
                    ps.join(" "),
                    tested
                ));
            }
            Err(reason) => out.push_str(&format!("FAIL  {:<34} {}\n", r.text, reason)),
        }
    }
    out.push_str(&format!("\n{}/{} templates learned\n", ok, results.len()));
    out
}

pub fn wto_json(name: &str, results: &[WShapeResult], end_byte: u8) -> String {
    let esc = |s: &str| s.replace('\\', "\\\\").replace('"', "\\\"");
    let mut out = format!(
        "{{\n  \"target\": \"{}\",\n  \"format\": \"bytes\",\n  \"end\": \"{:#04x}\",\n  \"instructions\": [\n",
        esc(name),
        end_byte
    );
    let mut first = true;
    for r in results {
        if let Ok((pieces, tested)) = &r.outcome {
            if !first {
                out.push_str(",\n");
            }
            first = false;
            let ps: Vec<String> = pieces
                .iter()
                .map(|p| match p {
                    Piece::Fixed(b) => format!(
                        "{{\"fixed\": \"{}\"}}",
                        b.iter().map(|x| format!("{:02x}", x)).collect::<String>()
                    ),
                    Piece::ULeb => "{\"uleb\": true}".into(),
                    Piece::SLeb => "{\"sleb\": true}".into(),
                    Piece::Bits64 => "{\"bits64\": true}".into(),
                    Piece::Bits32 => "{\"bits32\": true}".into(),
                })
                .collect();
            out.push_str(&format!(
                "    {{\"template\": \"{}\", \"verified\": {}, \"pieces\": [{}]}}",
                esc(&r.text),
                tested,
                ps.join(", ")
            ));
        }
    }
    out.push_str("\n  ]\n}\n");
    out
}

/// A seed file is byte-format if it declares `format bytes`.
pub fn is_bytes_seed(src: &str) -> bool {
    src.lines()
        .any(|l| l.split(';').next().unwrap().trim() == "format bytes")
}
