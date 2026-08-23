//! The learner: derives instruction encodings from an oracle, knowing
//! nothing about any particular ISA.
//!
//! Model: an instruction's encoding is a fixed bit pattern F XORed with one
//! contribution per operand slot, each a function of that slot's value only.
//! Each slot is probed one value at a time inside a *context* (the other
//! slots parked at constants), giving points `ctx ^ contribution(v)`; the
//! context constant cancels in deltas against the context's own v=0 point.
//! XORing points that differ in a single operand bit reveals which encoding
//! bit that operand bit controls — which handles scrambled fields (RISC-V
//! branch immediates) exactly as easily as contiguous ones.
//!
//! The default context parks other slots at 0. When the architecture makes
//! that invalid (`ldp x0, x0, [sp]` — same destination twice), the slot is
//! retried with other register slots parked at distinct high values, and F
//! is recovered afterwards by cancelling the parked slots' now-known
//! contributions. Every slot independently implies F, so slots must agree —
//! a free consistency check.
//!
//! Small domains (registers, enums) are probed exhaustively and stored as a
//! linear bit mapping if one fits, or a lookup table if not. Large domains
//! (immediates) are probed one-hot: N probes identify an N-bit field.
//!
//! Nothing is trusted until verified. Every learned shape is re-tested
//! against the oracle on random operand tuples, with immediate values
//! sampled *stratified by bit-width* (a width 1..=N chosen uniformly, then
//! a value of that width) — uniform sampling would almost never test small
//! immediates, which are both the common case and the ones assemblers
//! special-case. Anything nonlinear (e.g. ARM logical immediates) fails
//! cleanly and is reported as unlearned rather than silently mis-encoded.

use crate::oracle::Oracle;
use crate::target::{Shape, Slot, Target};

pub struct ShapeResult {
    pub text: String,
    pub outcome: Outcome,
}

pub enum Outcome {
    Learned {
        fixed: u64,
        fields: Vec<Field>,
        tested: usize,
        rejected: usize, // verification tuples the oracle refused (e.g. ldp same-reg)
    },
    Failed {
        reason: String,
    },
}

pub struct Field {
    pub slot: String, // display form of the slot spec
    pub kind: FieldKind,
}

pub enum FieldKind {
    /// bits[b] = encoding bit that operand bit b maps to.
    /// For signed fields the value is two's-complement in bits.len() bits.
    Linear { bits: Vec<u32>, signed: bool },
    /// entries[value] = this slot's contribution to the encoding.
    Table { entries: Vec<u64> },
}

impl FieldKind {
    fn apply(&self, value: i64) -> Option<u64> {
        match self {
            FieldKind::Linear { bits, signed } => {
                let n = bits.len() as u32;
                if n == 0 {
                    return if value == 0 { Some(0) } else { None };
                }
                let units = if *signed {
                    if value < -(1i64 << (n - 1)) || value >= (1i64 << (n - 1)) {
                        return None;
                    }
                    (value as u64) & ((1u64 << n) - 1)
                } else {
                    if value < 0 || (n < 64 && value as u64 >= 1u64 << n) {
                        return None;
                    }
                    value as u64
                };
                let mut out = 0u64;
                for (b, &enc_bit) in bits.iter().enumerate() {
                    if units >> b & 1 == 1 {
                        out |= 1u64 << enc_bit;
                    }
                }
                Some(out)
            }
            FieldKind::Table { entries } => entries.get(value as usize).copied(),
        }
    }

    fn mask(&self) -> u64 {
        match self {
            FieldKind::Linear { bits, .. } => bits.iter().fold(0, |m, &b| m | (1u64 << b)),
            FieldKind::Table { entries } => entries.iter().fold(0, |m, &e| m | e),
        }
    }
}

/// Contributions combine with XOR, not OR: each contribution is a delta
/// against the fixed pattern, and F may itself set bits inside a field
/// (e.g. cset's condition field, where value 0 = 'eq' encodes as inverted
/// 'ne' bits). XOR is exact in both cases.
pub fn predict(fixed: u64, fields: &[Field], values: &[i64]) -> Option<u64> {
    let mut enc = fixed;
    for (f, &v) in fields.iter().zip(values) {
        enc ^= f.kind.apply(v)?;
    }
    Some(enc)
}

// ---------------------------------------------------------------------------

/// Value domain of one slot, in "units" (reg/enum index, or immediate/step).
struct Domain {
    lo: i64,
    hi: i64,
}

impl Domain {
    fn exhaustive(&self) -> bool {
        self.hi - self.lo < EXHAUSTIVE_LIMIT
    }
}

fn domain(slot: &Slot, target: &Target) -> Domain {
    match slot {
        Slot::Reg { class } => Domain {
            lo: 0,
            hi: target.regs[class].len() as i64 - 1,
        },
        Slot::Enum { choices } => Domain {
            lo: 0,
            hi: choices.len() as i64 - 1,
        },
        Slot::Imm { lo, hi, step } => Domain {
            lo: lo / step,
            hi: hi / step,
        },
    }
}

const EXHAUSTIVE_LIMIT: i64 = 64;
const VERIFY_COUNT: usize = 64;
const VERIFY_MIN_ACCEPTED: usize = 16;

pub fn learn_target(target: &Target, oracle: &dyn Oracle) -> Result<Vec<ShapeResult>, String> {
    let mut results = Vec::new();
    for shape in &target.shapes {
        let outcome = learn_shape(target, shape, oracle)?;
        results.push(ShapeResult {
            text: shape.text.clone(),
            outcome,
        });
    }
    Ok(results)
}

fn to_u64(bytes: &[u8]) -> u64 {
    bytes
        .iter()
        .rev()
        .fold(0u64, |acc, &b| (acc << 8) | b as u64)
}

/// Probe values for one slot: 0 first (the context anchor), then the rest —
/// every value for small domains, one-hots (plus -1 if signed) for large.
fn probe_values(d: &Domain) -> Vec<i64> {
    let mut values = vec![0];
    if d.exhaustive() {
        values.extend((d.lo..=d.hi).filter(|&v| v != 0));
    } else {
        let mut b = 0;
        while 1i64 << b <= d.hi {
            values.push(1i64 << b);
            b += 1;
        }
        if d.lo < 0 {
            values.push(-1); // all-ones: reveals the full field incl. sign bits
        }
    }
    values
}

fn learn_shape(target: &Target, shape: &Shape, oracle: &dyn Oracle) -> Result<Outcome, String> {
    let slots = shape.slots();
    let n = slots.len();
    let domains: Vec<Domain> = slots.iter().map(|s| domain(s, target)).collect();

    for (slot, d) in slots.iter().zip(&domains) {
        if d.lo > 0 {
            return Ok(Outcome::Failed {
                reason: format!("slot {} can't take value 0 (lo > 0)", slot),
            });
        }
    }

    let width = target.width;

    // no operands: the rendered text itself is the whole encoding
    if n == 0 {
        let lines = vec![shape.render(target, &[])];
        let encs = oracle.assemble(&lines)?;
        return Ok(match encs[0].as_ref().filter(|b| b.len() == width) {
            Some(bytes) => Outcome::Learned {
                fixed: to_u64(bytes),
                fields: Vec::new(),
                tested: 1,
                rejected: 0,
            },
            None => Outcome::Failed {
                reason: format!("assembler rejected '{}'", lines[0]),
            },
        });
    }

    // the retry context: other *register* slots parked at distinct high
    // indices (immediates/enums stay at 0, they never make combos invalid)
    let mut park = vec![0i64; n];
    let mut ord = 0;
    for (j, slot) in slots.iter().enumerate() {
        if matches!(slot, Slot::Reg { .. }) {
            park[j] = (domains[j].hi - ord).max(0);
            ord += 1;
        }
    }

    // probe_batch: for each requested slot, render its probe values inside
    // a context (others parked at 0, or at `park` for the retry)
    let probe_batch = |slot_ids: &[usize], parked: bool| -> (Vec<String>, Vec<Vec<(i64, usize)>>) {
        let mut lines = Vec::new();
        let mut plan = Vec::new();
        for &k in slot_ids {
            let mut slot_plan = Vec::new();
            for v in probe_values(&domains[k]) {
                let assign: Vec<i64> = (0..n)
                    .map(|j| {
                        if j == k {
                            v
                        } else if parked {
                            park[j]
                        } else {
                            0
                        }
                    })
                    .collect();
                slot_plan.push((v, lines.len()));
                lines.push(shape.render(target, &assign));
            }
            plan.push(slot_plan);
        }
        (lines, plan)
    };

    // ---- probe every slot in the all-zeros context ----
    let all: Vec<usize> = (0..n).collect();
    let (lines_a, plan_a) = probe_batch(&all, false);
    let encs_a = oracle.assemble(&lines_a)?;

    let collect = |plan: &[(i64, usize)], lines: &[String], encs: &[Option<Vec<u8>>]| {
        let points: Vec<(i64, u64)> = plan
            .iter()
            .filter_map(|&(v, i)| {
                encs[i]
                    .as_ref()
                    .filter(|b| b.len() == width)
                    .map(|b| (v, to_u64(b)))
            })
            .collect();
        let example = plan
            .iter()
            .find(|&&(_, i)| encs[i].as_ref().filter(|b| b.len() == width).is_none())
            .map(|&(_, i)| lines[i].clone());
        (points, example)
    };

    // learned[k] = (field kind, encoding at v=0 in its context, used retry context?)
    let mut learned: Vec<Option<(FieldKind, u64, bool)>> =
        (0..n).map(|_| None).collect();
    let mut retry: Vec<usize> = Vec::new();
    let mut first_fail: Option<String> = None;
    for (k, plan) in plan_a.iter().enumerate() {
        let (points, example) = collect(plan, &lines_a, &encs_a);
        match learn_slot(&points, &domains[k]) {
            Ok(kind) => {
                let zero = points.iter().find(|&&(v, _)| v == 0).unwrap().1;
                learned[k] = Some((kind, zero, false));
            }
            Err(reason) => {
                retry.push(k);
                first_fail.get_or_insert(format!(
                    "slot {}: {}{}",
                    slots[k],
                    reason,
                    example.map_or(String::new(), |e| format!(" (e.g. '{}')", e))
                ));
            }
        }
    }

    // ---- retry failed slots in the parked context ----
    if !retry.is_empty() {
        let (lines_b, plan_b) = probe_batch(&retry, true);
        let encs_b = oracle.assemble(&lines_b)?;
        for (pi, &k) in retry.iter().enumerate() {
            let (points, example) = collect(&plan_b[pi], &lines_b, &encs_b);
            match learn_slot(&points, &domains[k]) {
                Ok(kind) => {
                    let Some(&(_, zero)) = points.iter().find(|&&(v, _)| v == 0) else {
                        return Ok(Outcome::Failed {
                            reason: first_fail.unwrap_or_else(|| {
                                format!("slot {}: no valid v=0 probe in any context", slots[k])
                            }),
                        });
                    };
                    learned[k] = Some((kind, zero, true));
                }
                Err(reason) => {
                    return Ok(Outcome::Failed {
                        reason: format!(
                            "slot {}: {}{}",
                            slots[k],
                            reason,
                            example.map_or(String::new(), |e| format!(" (e.g. '{}')", e))
                        ),
                    });
                }
            }
        }
    }
    let learned: Vec<(FieldKind, u64, bool)> = learned.into_iter().map(|l| l.unwrap()).collect();

    // ---- resolve the fixed pattern F ----
    // A slot probed in the zero context implies F = its v=0 encoding; one
    // probed in the parked context implies F = that, XOR the parked slots'
    // contributions (now known). All slots must agree.
    let mut f_candidates = Vec::new();
    for (k, (_, zero, parked)) in learned.iter().enumerate() {
        let mut f = *zero;
        if *parked {
            for j in 0..n {
                if j != k {
                    match learned[j].0.apply(park[j]) {
                        Some(c) => f ^= c,
                        None => {
                            return Ok(Outcome::Failed {
                                reason: format!(
                                    "slot {}: park value {} outside learned field",
                                    slots[j], park[j]
                                ),
                            })
                        }
                    }
                }
            }
        }
        f_candidates.push(f);
    }
    let fixed = f_candidates[0];
    if f_candidates.iter().any(|&f| f != fixed) {
        return Ok(Outcome::Failed {
            reason: "slots disagree on the fixed bits (fields are not independent)".into(),
        });
    }

    // ---- assemble fields; slots must not share encoding bits ----
    let mut fields = Vec::new();
    let mut field_mask = 0u64;
    for (k, (kind, _, _)) in learned.into_iter().enumerate() {
        let mask = kind.mask();
        if mask & field_mask != 0 {
            return Ok(Outcome::Failed {
                reason: format!("slot {} overlaps another slot's bits", slots[k]),
            });
        }
        field_mask |= mask;
        fields.push(Field {
            slot: slots[k].to_string(),
            kind,
        });
    }

    // ---- bulletproofing: random tuples, predicted vs oracle ----
    let mut rng = Rng(0x9E3779B97F4A7C15);
    let mut tuples: Vec<Vec<i64>> = Vec::new();
    tuples.push(domains.iter().map(|d| d.hi).collect()); // all-max corner
    if domains.iter().any(|d| d.lo < 0) {
        tuples.push(domains.iter().map(|d| d.lo).collect()); // all-min corner
    }
    for _ in 0..VERIFY_COUNT {
        tuples.push(domains.iter().map(|d| sample(d, &mut rng)).collect());
    }
    let vlines: Vec<String> = tuples.iter().map(|t| shape.render(target, t)).collect();
    let vencs = oracle.assemble(&vlines)?;
    let mut tested = 0;
    let mut rejected = 0;
    for (i, tuple) in tuples.iter().enumerate() {
        let actual = vencs[i].as_ref().filter(|b| b.len() == width).map(|b| to_u64(b));
        let Some(a) = actual else {
            rejected += 1; // architecturally invalid combination (e.g. ldp xN, xN)
            continue;
        };
        match predict(fixed, &fields, tuple) {
            Some(p) if p == a => tested += 1,
            p => {
                return Ok(Outcome::Failed {
                    reason: format!(
                        "verification mismatch on '{}': predicted {}, oracle {:#010x}",
                        vlines[i],
                        p.map_or("<out of range>".into(), |x| format!("{:#010x}", x)),
                        a
                    ),
                })
            }
        }
    }
    if tested < VERIFY_MIN_ACCEPTED {
        return Ok(Outcome::Failed {
            reason: format!(
                "only {} of {} verification probes were accepted by the assembler",
                tested,
                tuples.len()
            ),
        });
    }

    Ok(Outcome::Learned {
        fixed,
        fields,
        tested,
        rejected,
    })
}

/// Explain one slot's probe points as a field. Points are (value, encoding)
/// within a single context, and must include v=0 (the context anchor);
/// deltas against it are exactly this slot's contribution.
fn learn_slot(points: &[(i64, u64)], d: &Domain) -> Result<FieldKind, String> {
    let find = |v: i64| points.iter().find(|&&(pv, _)| pv == v).map(|&(_, e)| e);
    let Some(zero) = find(0) else {
        return Err("the v=0 probe was rejected".into());
    };
    if points.len() < 2 {
        return Err("almost all probes rejected".into());
    }
    let signed = d.lo < 0;

    // operand bit b: any two points whose values differ in exactly that bit
    let mut nbits = 0;
    while 1i64 << nbits <= d.hi {
        nbits += 1;
    }
    let mut bits: Vec<u32> = Vec::with_capacity(nbits);
    let mut linear = true;
    'per_bit: for b in 0..nbits {
        for &(v1, e1) in points {
            if v1 < 0 {
                continue;
            }
            if let Some(e2) = find(v1 ^ (1i64 << b)) {
                let delta = e1 ^ e2;
                if delta.count_ones() == 1 {
                    bits.push(delta.trailing_zeros());
                    continue 'per_bit;
                }
                linear = false; // one operand bit touches several encoding bits
                break 'per_bit;
            }
        }
        linear = false; // no probe pair isolates this bit
        break;
    }

    if linear && signed {
        // value -1 sets every field bit; those beyond the one-hot-probed ones
        // are the high/sign bits. Their internal order can't be observed from
        // probing alone, so assume ascending — for a wrong guess the random
        // verification pass rejects the whole shape.
        match find(-1) {
            Some(m1) => {
                let all_ones = m1 ^ zero;
                let known: u64 = bits.iter().fold(0, |m, &b| m | (1u64 << b));
                if all_ones & known != known {
                    linear = false; // -1 cleared a bit a one-hot set: not a plain field
                } else {
                    bits.extend((0..64).filter(|&b| all_ones >> b & 1 == 1 && known >> b & 1 == 0));
                }
            }
            None => linear = false,
        }
    }

    if linear {
        let kind = FieldKind::Linear { bits, signed };
        if points
            .iter()
            .all(|&(v, e)| kind.apply(v) == Some(e ^ zero))
        {
            return Ok(kind);
        }
    }

    // fall back to a lookup table: needs the whole domain probed successfully
    if d.exhaustive() && !signed {
        let mut entries = vec![u64::MAX; (d.hi + 1) as usize];
        for &(v, e) in points {
            entries[v as usize] = e ^ zero;
        }
        if let Some(missing) = entries.iter().position(|&e| e == u64::MAX) {
            return Err(format!(
                "nonlinear, and value {} was rejected so no full table",
                missing
            ));
        }
        return Ok(FieldKind::Table { entries });
    }
    Err(format!(
        "nonlinear {} encoding",
        if signed { "signed" } else { "immediate" }
    ))
}

/// Verification sampling. Small domains: uniform. Large domains: stratified
/// by bit-width — pick how many significant bits, then a value of that
/// width (negated half the time if the domain is signed). Uniform sampling
/// over a wide range almost never produces small values, and small
/// immediates are exactly where assemblers hide special cases.
fn sample(d: &Domain, rng: &mut Rng) -> i64 {
    if d.exhaustive() {
        return rng.range(d.lo, d.hi);
    }
    let mut nbits = 0;
    while 1i64 << nbits <= d.hi {
        nbits += 1;
    }
    let width = rng.range(1, nbits as i64) as u32;
    let lo_of_width = if width == 1 { 1 } else { 1i64 << (width - 1) };
    let hi_of_width = ((1i64 << width) - 1).min(d.hi);
    let mut v = rng.range(lo_of_width, hi_of_width);
    if d.lo < 0 && rng.next() & 1 == 1 {
        v = (-v).max(d.lo);
    }
    v
}

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        // xorshift64*
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545F4914F6CDD1D)
    }

    fn range(&mut self, lo: i64, hi: i64) -> i64 {
        let span = (hi - lo) as u64 + 1;
        lo + (self.next() % span) as i64
    }
}

// ---------------------------------------------------------------------------
// Reporting and serialization

pub fn report(results: &[ShapeResult]) -> String {
    let mut out = String::new();
    let learned = results
        .iter()
        .filter(|r| matches!(r.outcome, Outcome::Learned { .. }))
        .count();
    for r in results {
        match &r.outcome {
            Outcome::Learned {
                fixed,
                fields,
                tested,
                rejected,
            } => {
                let fs = fields
                    .iter()
                    .map(|f| match &f.kind {
                        FieldKind::Linear { bits, signed } => {
                            let s = if *signed { "s" } else { "" };
                            if contiguous(bits) {
                                format!("{}[{}:{}]{}", f.slot, bits[0], bits[bits.len() - 1] + 1, s)
                            } else {
                                format!("{}[scrambled:{}b]{}", f.slot, bits.len(), s)
                            }
                        }
                        FieldKind::Table { entries } => {
                            format!("{}[table:{}]", f.slot, entries.len())
                        }
                    })
                    .collect::<Vec<_>>()
                    .join(" ");
                let rej = if *rejected > 0 {
                    format!(", {} rejected", rejected)
                } else {
                    String::new()
                };
                out.push_str(&format!(
                    "  ok  {:<40} fixed={:#010x}  {}  (verified {}{})\n",
                    r.text, fixed, fs, tested, rej
                ));
            }
            Outcome::Failed { reason } => {
                out.push_str(&format!("FAIL  {:<40} {}\n", r.text, reason));
            }
        }
    }
    out.push_str(&format!(
        "\n{}/{} shapes learned and verified\n",
        learned,
        results.len()
    ));
    out
}

fn contiguous(bits: &[u32]) -> bool {
    !bits.is_empty() && bits.windows(2).all(|w| w[1] == w[0] + 1)
}

pub fn to_json(target: &Target, results: &[ShapeResult]) -> String {
    let esc = |s: &str| s.replace('\\', "\\\\").replace('"', "\\\"");
    let mut out = String::new();
    out.push_str(&format!(
        "{{\n  \"target\": \"{}\",\n  \"triple\": \"{}\",\n  \"width\": {},\n  \"combine\": \"xor\",\n  \"instructions\": [\n",
        esc(&target.name),
        esc(&target.triple),
        target.width
    ));
    let mut first = true;
    for r in results {
        if let Outcome::Learned {
            fixed,
            fields,
            tested,
            rejected,
        } = &r.outcome
        {
            if !first {
                out.push_str(",\n");
            }
            first = false;
            out.push_str(&format!(
                "    {{\"template\": \"{}\", \"fixed\": \"{:#010x}\", \"verified\": {}, \"rejected\": {}, \"fields\": [",
                esc(&r.text),
                fixed,
                tested,
                rejected
            ));
            for (i, f) in fields.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                match &f.kind {
                    FieldKind::Linear { bits, signed } => out.push_str(&format!(
                        "{{\"slot\": \"{}\", \"kind\": \"linear\", \"signed\": {}, \"bits\": {:?}}}",
                        esc(&f.slot),
                        signed,
                        bits
                    )),
                    FieldKind::Table { entries } => {
                        let es: Vec<String> =
                            entries.iter().map(|e| format!("\"{:#x}\"", e)).collect();
                        out.push_str(&format!(
                            "{{\"slot\": \"{}\", \"kind\": \"table\", \"entries\": [{}]}}",
                            esc(&f.slot),
                            es.join(", ")
                        ));
                    }
                }
            }
            out.push_str("]}");
        }
    }
    out.push_str("\n  ]\n}\n");
    out
}
