//! Wide values: integers and packs of 65 to 256 bits, lowered to words.
//!
//! No backend sees a value wider than 64 bits. Right after parsing, a
//! function that uses one is rewritten so that each wide value is a row
//! of 64-bit words, lowest first — the value's own id becomes word 0 and
//! fresh values `x.1`, `x.2`... the rest; the top word keeps the type's
//! signedness and its remaining bits (`u36` for a `u100`), so the
//! ordinary canonical form extends it. Every instruction on a wide value
//! becomes the narrow instructions that compute its words: carry chains
//! for `add`/`sub`, schoolbook products for `mul`, word-and-bit
//! arrangement for constant shifts and a logarithmic select for variable
//! ones, lexicographic compares, sign-filled extensions. `div` and `rem`
//! are the one exception: the parser dispatches them to the library's
//! `div(W)`/`rem(W)` generics in lib/wide.ssa, whose loops are written in
//! SSA over the wide type and lowered like anything else.

use crate::ssa::{BinOp, CastOp, Cond, Function, Inst, Module, Type, ValueData, ValueId};
use std::collections::HashMap;

pub const MAX_BITS: u32 = 256;

/// the words a wide type splits into: u64s and a typed top word
pub fn word_types(f: &Function, ty: Type) -> Vec<Type> {
    let w = f.width(ty).unwrap_or(64);
    if w <= 64 {
        return vec![ty];
    }
    let n = w.div_ceil(64);
    let signed = matches!(ty, Type::Int { signed: true, .. });
    let mut v = vec![Type::U64; (n - 1) as usize];
    v.push(Type::int(signed, w - 64 * (n - 1)));
    v
}

fn is_wide(f: &Function, ty: Type) -> bool {
    f.width(ty).map_or(false, |w| w > 64)
}

/// does the module have anything to lower?
pub fn has_wide(m: &Module) -> bool {
    m.funcs.iter().any(|f| f.values.iter().any(|v| is_wide(f, v.ty)) || f.rets.iter().any(|&t| is_wide(f, t)))
}

pub fn lower(m: &mut Module) -> Result<(), String> {
    for f in &mut m.funcs {
        if f.values.iter().any(|v| is_wide(f, v.ty)) || f.rets.iter().any(|&t| is_wide(f, t)) {
            lower_function(f).map_err(|e| format!("{}: {}", f.name, e))?;
        }
    }
    Ok(())
}

struct Lower<'a> {
    f: &'a mut Function,
    /// wide value -> its words (word 0 is the value itself)
    words: HashMap<u32, Vec<ValueId>>,
    /// the original types of the wide values (their ids are retyped)
    orig: HashMap<u32, Type>,
    /// constants, for shifts by a known amount
    consts: HashMap<u32, i128>,
    out: Vec<Inst>,
}

fn lower_function(f: &mut Function) -> Result<(), String> {
    f.wide_sig = Some((f.params.iter().map(|&p| f.ty(p)).collect(), f.rets.clone()));
    // rets first: word types of the declared results
    let mut rets = Vec::new();
    for &t in &f.rets {
        rets.extend(word_types(f, t));
    }
    f.rets = rets;
    let mut lo = Lower { f, words: HashMap::new(), orig: HashMap::new(), consts: HashMap::new(), out: Vec::new() };
    // every wide value gets its words up front
    let n = lo.f.values.len();
    for i in 0..n {
        let id = ValueId(i as u32);
        let ty = lo.f.ty(id);
        if !is_wide(lo.f, ty) {
            continue;
        }
        let wts = word_types(lo.f, ty);
        let name = lo.f.values[i].name.clone();
        let mut ws = vec![id];
        for (k, &wt) in wts.iter().enumerate().skip(1) {
            ws.push(lo.fresh(wt, format!("{}.{}", name, k)));
        }
        lo.f.values[i].ty = Type::U64;
        lo.f.values[i].literal = None;
        lo.orig.insert(id.0, ty);
        lo.words.insert(id.0, ws);
    }
    // parameters and block parameters: a wide one is its words in place
    let params = std::mem::take(&mut lo.f.params);
    lo.f.params = lo.expand_list(&params);
    let nblocks = lo.f.blocks.len();
    for b in 0..nblocks {
        let ps = std::mem::take(&mut lo.f.blocks[b].params);
        lo.f.blocks[b].params = lo.expand_list(&ps);
        let insts = std::mem::take(&mut lo.f.blocks[b].insts);
        for inst in insts {
            lo.inst(inst)?;
        }
        lo.f.blocks[b].insts = std::mem::take(&mut lo.out);
    }
    Ok(())
}

impl Lower<'_> {
    fn fresh(&mut self, ty: Type, name: String) -> ValueId {
        self.f.values.push(ValueData { name, ty, literal: None });
        ValueId(self.f.values.len() as u32 - 1)
    }

    fn tmp(&mut self, ty: Type) -> ValueId {
        let n = self.f.values.len();
        self.fresh(ty, format!("w{}", n))
    }

    fn wide(&self, v: ValueId) -> bool {
        self.words.contains_key(&v.0)
    }

    /// the declared-type words of a value (a narrow value is itself)
    fn expand(&self, v: ValueId) -> Vec<ValueId> {
        self.words.get(&v.0).cloned().unwrap_or_else(|| vec![v])
    }

    fn expand_list(&self, vs: &[ValueId]) -> Vec<ValueId> {
        vs.iter().flat_map(|&v| self.expand(v)).collect()
    }

    fn orig_ty(&self, v: ValueId) -> Type {
        self.orig.get(&v.0).copied().unwrap_or_else(|| self.f.ty(v))
    }

    fn signed(&self, v: ValueId) -> bool {
        matches!(self.orig_ty(v), Type::Int { signed: true, .. })
    }

    // --- narrow instruction builders ---

    fn cst(&mut self, ty: Type, imm: i64) -> ValueId {
        let d = self.tmp(ty);
        self.out.push(Inst::IConst { dst: d, imm: imm as i128 });
        d
    }

    fn bin(&mut self, op: BinOp, a: ValueId, b: ValueId) -> ValueId {
        let d = self.tmp(self.f.ty(a));
        self.out.push(Inst::Bin { op, dst: d, lhs: a, rhs: b });
        d
    }

    fn bin_imm(&mut self, op: BinOp, a: ValueId, imm: i64) -> ValueId {
        let k = self.cst(self.f.ty(a), imm);
        self.bin(op, a, k)
    }

    fn cmp(&mut self, cond: Cond, a: ValueId, b: ValueId) -> ValueId {
        let d = self.tmp(Type::U1);
        self.out.push(Inst::ICmp { cond, dst: d, lhs: a, rhs: b });
        d
    }

    fn conv(&mut self, ty: Type, a: ValueId) -> ValueId {
        let d = self.tmp(ty);
        self.out.push(Inst::Cast { op: CastOp::Conv, dst: d, src: a });
        d
    }

    fn conv_into(&mut self, dst: ValueId, a: ValueId) {
        self.out.push(Inst::Cast { op: CastOp::Conv, dst, src: a });
    }

    fn cast(&mut self, ty: Type, a: ValueId) -> ValueId {
        let d = self.tmp(ty);
        self.out.push(Inst::Cast { op: CastOp::Cast, dst: d, src: a });
        d
    }

    /// the u64 views of a wide value's words (the top word extended)
    fn views(&mut self, v: ValueId) -> Vec<ValueId> {
        let ws = self.expand(v);
        let n = ws.len();
        let mut out = ws[..n - 1].to_vec();
        let top = ws[n - 1];
        out.push(if self.f.ty(top) == Type::U64 { top } else { self.conv(Type::U64, top) });
        out
    }

    /// the unsigned bit pattern of any value, as u64 words
    fn bits(&mut self, v: ValueId) -> Vec<ValueId> {
        if self.wide(v) {
            let ws = self.expand(v);
            let n = ws.len();
            let mut out = ws[..n - 1].to_vec();
            let top = ws[n - 1];
            let tw = self.f.width(self.f.ty(top)).unwrap();
            let u = if tw == 64 && self.f.ty(top) == Type::U64 { top } else { self.cast(Type::int(false, tw), top) };
            out.push(if tw == 64 { u } else { self.conv(Type::U64, u) });
            out
        } else {
            let ty = self.f.ty(v);
            let w = self.f.width(ty).unwrap_or(64);
            let u = if ty == Type::U64 { v } else if ty == Type::Ptr { self.cast(Type::U64, v) } else { self.cast(Type::int(false, w), v) };
            vec![if w == 64 { u } else { self.conv(Type::U64, u) }]
        }
    }

    /// all-ones when the (sign-extended) u64 view is negative, else zero
    fn sign_fill(&mut self, top_view: ValueId) -> ValueId {
        let s = self.cast(Type::I64, top_view);
        let sh = self.bin_imm(BinOp::Shr, s, 63);
        self.cast(Type::U64, sh)
    }

    /// define a wide value's words from u64 values
    fn define(&mut self, dst: ValueId, vals: &[ValueId]) {
        let ws = self.expand(dst);
        for (w, v) in ws.iter().zip(vals) {
            self.conv_into(*w, *v);
        }
    }

    // --- the arithmetic on u64 word rows ---

    fn add_row(&mut self, a: &[ValueId], b: &[ValueId]) -> Vec<ValueId> {
        let n = a.len();
        let mut out = Vec::new();
        let mut carry: Option<ValueId> = None;
        for i in 0..n {
            let t = self.bin(BinOp::IAdd, a[i], b[i]);
            let (s, c) = match carry {
                None => {
                    let c = self.cmp(Cond::Lt, t, a[i]);
                    (t, c)
                }
                Some(c) => {
                    let cw = self.conv(Type::U64, c);
                    let s = self.bin(BinOp::IAdd, t, cw);
                    let c1 = self.cmp(Cond::Lt, t, a[i]);
                    let c2 = self.cmp(Cond::Lt, s, t);
                    let c = self.bin(BinOp::Or, c1, c2);
                    (s, c)
                }
            };
            out.push(s);
            carry = Some(c);
        }
        out
    }

    fn sub_row(&mut self, a: &[ValueId], b: &[ValueId]) -> Vec<ValueId> {
        let n = a.len();
        let mut out = Vec::new();
        let mut borrow: Option<ValueId> = None;
        for i in 0..n {
            let t = self.bin(BinOp::ISub, a[i], b[i]);
            let (s, br) = match borrow {
                None => {
                    let br = self.cmp(Cond::Lt, a[i], b[i]);
                    (t, br)
                }
                Some(br) => {
                    let bw = self.conv(Type::U64, br);
                    let s = self.bin(BinOp::ISub, t, bw);
                    let b1 = self.cmp(Cond::Lt, a[i], b[i]);
                    let b2 = self.cmp(Cond::Lt, t, bw);
                    let br = self.bin(BinOp::Or, b1, b2);
                    (s, br)
                }
            };
            out.push(s);
            borrow = Some(br);
        }
        out
    }

    /// the 128-bit product of two u64s as (lo, hi), from 32-bit halves
    fn mul64(&mut self, x: ValueId, y: ValueId) -> (ValueId, ValueId) {
        let m32 = 0xffff_ffffi64;
        let xl = self.bin_imm(BinOp::And, x, m32);
        let xh = self.bin_imm(BinOp::Shr, x, 32);
        let yl = self.bin_imm(BinOp::And, y, m32);
        let yh = self.bin_imm(BinOp::Shr, y, 32);
        let ll = self.bin(BinOp::IMul, xl, yl);
        let lh = self.bin(BinOp::IMul, xl, yh);
        let hl = self.bin(BinOp::IMul, xh, yl);
        let hh = self.bin(BinOp::IMul, xh, yh);
        let ll_hi = self.bin_imm(BinOp::Shr, ll, 32);
        let lh_lo = self.bin_imm(BinOp::And, lh, m32);
        let hl_lo = self.bin_imm(BinOp::And, hl, m32);
        let mid0 = self.bin(BinOp::IAdd, ll_hi, lh_lo);
        let mid = self.bin(BinOp::IAdd, mid0, hl_lo);
        let ll_lo = self.bin_imm(BinOp::And, ll, m32);
        let mid_sh = self.bin_imm(BinOp::Shl, mid, 32);
        let lo = self.bin(BinOp::Or, ll_lo, mid_sh);
        let lh_hi = self.bin_imm(BinOp::Shr, lh, 32);
        let hl_hi = self.bin_imm(BinOp::Shr, hl, 32);
        let mid_hi = self.bin_imm(BinOp::Shr, mid, 32);
        let h0 = self.bin(BinOp::IAdd, hh, lh_hi);
        let h1 = self.bin(BinOp::IAdd, h0, hl_hi);
        let hi = self.bin(BinOp::IAdd, h1, mid_hi);
        (lo, hi)
    }

    /// add `v` into word `k` of an accumulator, carrying upward
    fn acc_add(&mut self, acc: &mut [Option<ValueId>], k: usize, v: ValueId) {
        if k >= acc.len() {
            return;
        }
        match acc[k] {
            None => acc[k] = Some(v),
            Some(cur) => {
                let s = self.bin(BinOp::IAdd, cur, v);
                acc[k] = Some(s);
                if k + 1 < acc.len() {
                    let c = self.cmp(Cond::Lt, s, cur);
                    let cw = self.conv(Type::U64, c);
                    self.acc_add(acc, k + 1, cw);
                }
            }
        }
    }

    fn mul_row(&mut self, a: &[ValueId], b: &[ValueId]) -> Vec<ValueId> {
        let n = a.len();
        let mut acc: Vec<Option<ValueId>> = vec![None; n];
        for i in 0..n {
            for j in 0..n - i {
                let (lo, hi) = self.mul64(a[i], b[j]);
                self.acc_add(&mut acc, i + j, lo);
                self.acc_add(&mut acc, i + j + 1, hi);
            }
        }
        acc.into_iter().map(|w| w.unwrap_or_else(|| self.cst(Type::U64, 0))).collect::<Vec<_>>()
    }

    /// shift a row by a constant: `ext` is the row followed by what comes
    /// in from beyond its top (its sign fill, or zeros), and from below
    /// zeros
    fn shl_const(&mut self, w: &[ValueId], k: u32) -> Vec<ValueId> {
        let n = w.len();
        let (q, r) = ((k / 64) as usize, k % 64);
        let mut out = Vec::new();
        for i in 0..n {
            let mut acc: Option<ValueId> = None;
            if i >= q {
                acc = Some(if r == 0 { w[i - q] } else { self.bin_imm(BinOp::Shl, w[i - q], r as i64) });
            }
            if r != 0 && i >= q + 1 {
                let lo = self.bin_imm(BinOp::Shr, w[i - q - 1], (64 - r) as i64);
                acc = Some(match acc {
                    Some(a) => self.bin(BinOp::Or, a, lo),
                    None => lo,
                });
            }
            out.push(acc.unwrap_or_else(|| self.cst(Type::U64, 0)));
        }
        out
    }

    fn shr_const(&mut self, w: &[ValueId], k: u32, fill: ValueId) -> Vec<ValueId> {
        let n = w.len();
        let (q, r) = ((k / 64) as usize, k % 64);
        let ext = |i: usize| if i < n { w[i] } else { fill };
        let mut out = Vec::new();
        for i in 0..n {
            let hi_src = ext(i + q);
            let a = if r == 0 { hi_src } else { self.bin_imm(BinOp::Shr, hi_src, r as i64) };
            let v = if r == 0 {
                a
            } else {
                let up = ext(i + q + 1);
                let b = self.bin_imm(BinOp::Shl, up, (64 - r) as i64);
                self.bin(BinOp::Or, a, b)
            };
            out.push(v);
        }
        out
    }

    /// a shift by a runtime amount (its low word): for each power of two
    /// below the width, the row shifted by it if that bit of the amount
    /// is set, selected without branching
    fn shift_var(&mut self, w: &[ValueId], amount: ValueId, left: bool, fill: ValueId, width: u32) -> Vec<ValueId> {
        let mut cur = w.to_vec();
        let mut p = 1u32;
        let mut log = 0i64;
        while p < width {
            let shifted = if left { self.shl_const(&cur, p) } else { self.shr_const(&cur, p, fill) };
            let bit0 = self.bin_imm(BinOp::Shr, amount, log);
            let bit = self.bin_imm(BinOp::And, bit0, 1);
            let zero = self.cst(Type::U64, 0);
            let m = self.bin(BinOp::ISub, zero, bit);
            let nm = self.bin_imm(BinOp::Xor, m, -1);
            let mut next = Vec::new();
            for i in 0..cur.len() {
                let a = self.bin(BinOp::And, shifted[i], m);
                let b = self.bin(BinOp::And, cur[i], nm);
                next.push(self.bin(BinOp::Or, a, b));
            }
            cur = next;
            p *= 2;
            log += 1;
        }
        cur
    }

    /// a < b over rows, the top words compared as their own types
    fn lt_row(&mut self, a: ValueId, b: ValueId) -> ValueId {
        let (aw, bw) = (self.expand(a), self.expand(b));
        let n = aw.len();
        let mut r = self.cmp(Cond::Lt, aw[0], bw[0]);
        for i in 1..n {
            let e = self.cmp(Cond::Eq, aw[i], bw[i]);
            let l = self.cmp(Cond::Lt, aw[i], bw[i]);
            let t = self.bin(BinOp::And, e, r);
            r = self.bin(BinOp::Or, l, t);
        }
        r
    }

    fn eq_row(&mut self, a: ValueId, b: ValueId) -> ValueId {
        let (av, bv) = (self.views(a), self.views(b));
        let mut acc = self.bin(BinOp::Xor, av[0], bv[0]);
        for i in 1..av.len() {
            let x = self.bin(BinOp::Xor, av[i], bv[i]);
            acc = self.bin(BinOp::Or, acc, x);
        }
        let z = self.cst(Type::U64, 0);
        self.cmp(Cond::Eq, acc, z)
    }

    /// the row of a pack field's bits, placed at its offset
    fn place(&mut self, v: ValueId, off: u32, n: usize) -> Vec<ValueId> {
        let mut bits = self.bits(v);
        while bits.len() < n {
            bits.push(self.cst(Type::U64, 0));
        }
        bits.truncate(n);
        self.shl_const(&bits, off)
    }

    /// the words with ones outside [off, off + w)
    fn keep_mask(&mut self, off: u32, w: u32, n: usize) -> Vec<ValueId> {
        let mut out = Vec::new();
        for i in 0..n {
            let (lo, hi) = (64 * i as u32, 64 * i as u32 + 64);
            let (s, e) = (off.max(lo), (off + w).min(hi));
            let field = if s < e { ((1u128 << (e - s)) - 1) << (s - lo) } else { 0 };
            out.push(self.cst(Type::U64, !(field as u64) as i64));
        }
        out
    }

    fn inst(&mut self, inst: Inst) -> Result<(), String> {
        match inst {
            Inst::IConst { dst, imm } if self.wide(dst) => {
                // the 128-bit constant's words, then its sign beyond
                self.consts.insert(dst.0, imm);
                let ws = self.expand(dst);
                for (k, &w) in ws.iter().enumerate() {
                    let word = if k < 2 { (imm >> (64 * k)) as i64 } else { (imm >> 127) as i64 };
                    self.out.push(Inst::IConst { dst: w, imm: word as i128 });
                }
            }
            Inst::IConst { dst, imm } => {
                self.consts.insert(dst.0, imm);
                self.out.push(Inst::IConst { dst, imm });
            }
            Inst::Bin { op, dst, lhs, rhs } if self.wide(dst) => {
                let width = self.f.width(self.orig_ty(dst)).unwrap();
                let a = self.views(lhs);
                let row = match op {
                    BinOp::IAdd => {
                        let b = self.views(rhs);
                        self.add_row(&a, &b)
                    }
                    BinOp::ISub => {
                        let b = self.views(rhs);
                        self.sub_row(&a, &b)
                    }
                    BinOp::IMul => {
                        let b = self.views(rhs);
                        self.mul_row(&a, &b)
                    }
                    BinOp::And | BinOp::Or | BinOp::Xor => {
                        let b = self.views(rhs);
                        (0..a.len()).map(|i| self.bin(op, a[i], b[i])).collect()
                    }
                    BinOp::Shl | BinOp::Shr => {
                        let n = a.len();
                        let fill = if op == BinOp::Shr && self.signed(dst) { self.sign_fill(a[n - 1]) } else { self.cst(Type::U64, 0) };
                        match self.consts.get(&rhs.0).copied() {
                            Some(k) if k >= 0 && k < width as i128 => {
                                if op == BinOp::Shl { self.shl_const(&a, k as u32) } else { self.shr_const(&a, k as u32, fill) }
                            }
                            Some(_) => return Err("shift by an amount outside the width".into()),
                            None => {
                                let amt = self.expand(rhs)[0];
                                self.shift_var(&a, amt, op == BinOp::Shl, fill, width)
                            }
                        }
                    }
                    BinOp::Div | BinOp::Rem => return Err("wide div/rem should have been dispatched to the library".into()),
                };
                self.define(dst, &row);
            }
            Inst::ICmp { cond, dst, lhs, rhs } if self.wide(lhs) => {
                let r = match cond {
                    Cond::Eq => self.eq_row(lhs, rhs),
                    Cond::Ne => {
                        let e = self.eq_row(lhs, rhs);
                        self.bin_imm(BinOp::Xor, e, 1)
                    }
                    Cond::Lt => self.lt_row(lhs, rhs),
                    Cond::Gt => self.lt_row(rhs, lhs),
                    Cond::Le => {
                        let l = self.lt_row(rhs, lhs);
                        self.bin_imm(BinOp::Xor, l, 1)
                    }
                    Cond::Ge => {
                        let l = self.lt_row(lhs, rhs);
                        self.bin_imm(BinOp::Xor, l, 1)
                    }
                };
                self.conv_into(dst, r);
            }
            Inst::Cast { op: CastOp::Conv, dst, src } if self.wide(dst) || self.wide(src) => {
                let n = self.expand(dst).len();
                if !self.wide(src) {
                    // narrow to wide: extend by the source's signedness
                    let w0 = if self.f.ty(src) == Type::U64 { src } else { self.conv(Type::U64, src) };
                    let fill = if self.signed(src) { self.sign_fill(w0) } else { self.cst(Type::U64, 0) };
                    let mut row = vec![w0];
                    row.resize(n, fill);
                    self.define(dst, &row);
                } else if !self.wide(dst) {
                    let v = self.views(src);
                    self.conv_into(dst, v[0]);
                } else {
                    let mut v = self.views(src);
                    if v.len() < n {
                        let fill = if self.signed(src) { self.sign_fill(*v.last().unwrap()) } else { self.cst(Type::U64, 0) };
                        v.resize(n, fill);
                    }
                    v.truncate(n);
                    self.define(dst, &v);
                }
            }
            Inst::Cast { op: CastOp::Cast, dst, src } if self.wide(dst) => {
                let v = self.bits(src);
                self.define(dst, &v);
            }
            Inst::Get { dst, src, field } if self.wide(src) => {
                let (off, _) = self.f.field(self.orig_ty(src), field).unwrap();
                let v = self.views(src);
                let zero = self.cst(Type::U64, 0);
                let sh = self.shr_const(&v, off, zero);
                if self.wide(dst) {
                    let n = self.expand(dst).len();
                    self.define(dst, &sh[..n]);
                } else {
                    self.conv_into(dst, sh[0]);
                }
            }
            Inst::Unpack { dsts, src } if self.wide(src) => {
                for (i, d) in dsts.iter().enumerate() {
                    self.inst(Inst::Get { dst: *d, src, field: i as u32 })?;
                }
            }
            Inst::Set { dst, src, field, val } if self.wide(dst) => {
                let (off, fty) = self.f.field(self.orig_ty(src), field).unwrap();
                let fw = self.f.width(fty).unwrap();
                let v = self.views(src);
                let n = v.len();
                let keep = self.keep_mask(off, fw, n);
                let placed = self.place(val, off, n);
                let row: Vec<ValueId> = (0..n)
                    .map(|i| {
                        let k = self.bin(BinOp::And, v[i], keep[i]);
                        self.bin(BinOp::Or, k, placed[i])
                    })
                    .collect();
                self.define(dst, &row);
            }
            Inst::Pack { dst, args } if self.wide(dst) => {
                let n = self.expand(dst).len();
                let ty = self.orig_ty(dst);
                let mut row: Vec<Option<ValueId>> = vec![None; n];
                for (i, &a) in args.iter().enumerate() {
                    let (off, _) = self.f.field(ty, i as u32).unwrap();
                    let placed = self.place(a, off, n);
                    for k in 0..n {
                        row[k] = Some(match row[k] {
                            None => placed[k],
                            Some(cur) => self.bin(BinOp::Or, cur, placed[k]),
                        });
                    }
                }
                let row: Vec<ValueId> = row.into_iter().map(|w| w.unwrap_or_else(|| self.cst(Type::U64, 0))).collect();
                self.define(dst, &row);
            }
            Inst::Load { dst, addr, off, index } if self.wide(dst) => {
                let n = self.expand(dst).len();
                let mut row = Vec::new();
                for i in 0..n {
                    let w = self.tmp(Type::U64);
                    self.out.push(Inst::Load { dst: w, addr, off: off + 8 * i as i64, index });
                    row.push(w);
                }
                self.define(dst, &row);
            }
            Inst::Store { val, addr, off, index } if self.wide(val) => {
                let v = self.views(val);
                for (i, &w) in v.iter().enumerate() {
                    self.out.push(Inst::Store { val: w, addr, off: off + 8 * i as i64, index });
                }
            }
            Inst::Call { dsts, callee, args } => {
                let dsts = self.expand_list(&dsts);
                let args = self.expand_list(&args);
                self.out.push(Inst::Call { dsts, callee, args });
            }
            Inst::Jmp { target, args } => {
                let args = self.expand_list(&args);
                self.out.push(Inst::Jmp { target, args });
            }
            Inst::Br { cond, then_target, then_args, else_target, else_args } => {
                let then_args = self.expand_list(&then_args);
                let else_args = self.expand_list(&else_args);
                self.out.push(Inst::Br { cond, then_target, then_args, else_target, else_args });
            }
            Inst::Ret { vals } => {
                let vals = self.expand_list(&vals);
                self.out.push(Inst::Ret { vals });
            }
            other => self.out.push(other),
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::emit::{self, jit::JitCode};
    use crate::platform::Platform;
    use crate::ssa::{self, Policy, Type};

    struct Rng(u64);
    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }
        /// a 128-bit value with edges and sparse patterns often
        fn wide(&mut self) -> u128 {
            let r = self.next();
            let v = ((self.next() as u128) << 64) | self.next() as u128;
            match r % 8 {
                0 => 0,
                1 => u128::MAX,
                2 => 1u128 << (self.next() % 128),
                3 => (1u128 << (self.next() % 128)) - 1,
                4 => v as u64 as u128,
                5 => v & ((1u128 << 64) - 1) << 64,
                _ => v,
            }
        }
    }

    const SRC: &str = "
fn add(a: u128, b: u128) -> u128 {
    r: u128 = add a, b
    ret r
}
fn sub(a: u128, b: u128) -> u128 {
    r: u128 = sub a, b
    ret r
}
fn mul(a: u128, b: u128) -> u128 {
    r: u128 = mul a, b
    ret r
}
fn div(a: u128, b: u128) -> u128 {
    r: u128 = div a, b
    ret r
}
fn rem(a: u128, b: u128) -> u128 {
    r: u128 = rem a, b
    ret r
}
fn sdiv(a: i128, b: i128) -> i128 {
    r: i128 = div a, b
    ret r
}
fn srem(a: i128, b: i128) -> i128 {
    r: i128 = rem a, b
    ret r
}
fn shl(a: u128, b: u128) -> u128 {
    r: u128 = shl a, b
    ret r
}
fn shr(a: u128, b: u128) -> u128 {
    r: u128 = shr a, b
    ret r
}
fn sar(a: i128, b: i128) -> i128 {
    r: i128 = shr a, b
    ret r
}
fn lt(a: u128, b: u128) -> u1 {
    r: u1 = cmp.lt a, b
    ret r
}
fn slt(a: i128, b: i128) -> u1 {
    r: u1 = cmp.lt a, b
    ret r
}
fn to_i32(a: i128) -> i32 {
    r: i32 = conv a
    ret r
}
fn from_i32(a: i32) -> i128 {
    r: i128 = conv a
    ret r
}
";

    fn split(v: u128) -> [i64; 2] {
        [v as u64 as i64, (v >> 64) as u64 as i64]
    }

    /// every wide operation against Rust's u128/i128 on random rows
    #[test]
    fn wide_arithmetic_matches_rust_u128() {
        let enc = emit::Encoder::load("targets/arm64.encodings.json").unwrap();
        let module = ssa::parse_with(&ssa::with_prelude(SRC), &Policy::new(Type::I64).unwrap()).unwrap();
        ssa::verify(&module).unwrap();
        let jit = JitCode::new(&emit::compile_with(&module, &enc, &Platform::none()).unwrap()).unwrap();
        let mut rng = Rng(0x2545_f491_4f6c_dd1d);
        for _ in 0..400 {
            let (a, b) = (rng.wide(), rng.wide());
            let args: Vec<i64> = split(a).iter().chain(split(b).iter()).copied().collect();
            let two = |name: &str| -> u128 {
                let (lo, hi) = jit.call2(name, &args).unwrap();
                (lo as u64 as u128) | ((hi as u64 as u128) << 64)
            };
            assert_eq!(two("add"), a.wrapping_add(b), "add {:x} {:x}", a, b);
            assert_eq!(two("sub"), a.wrapping_sub(b), "sub {:x} {:x}", a, b);
            assert_eq!(two("mul"), a.wrapping_mul(b), "mul {:x} {:x}", a, b);
            if b != 0 {
                assert_eq!(two("div"), a / b, "div {:x} {:x}", a, b);
                assert_eq!(two("rem"), a % b, "rem {:x} {:x}", a, b);
                let (sa, sb) = (a as i128, b as i128);
                assert_eq!(two("sdiv") as i128, sa.wrapping_div(sb), "sdiv {:x} {:x}", a, b);
                assert_eq!(two("srem") as i128, sa.wrapping_rem(sb), "srem {:x} {:x}", a, b);
            }
            let k = b % 128;
            let kargs: Vec<i64> = split(a).iter().chain(split(k).iter()).copied().collect();
            let two_k = |name: &str| -> u128 {
                let (lo, hi) = jit.call2(name, &kargs).unwrap();
                (lo as u64 as u128) | ((hi as u64 as u128) << 64)
            };
            assert_eq!(two_k("shl"), a << k, "shl {:x} by {}", a, k);
            assert_eq!(two_k("shr"), a >> k, "shr {:x} by {}", a, k);
            assert_eq!(two_k("sar") as i128, (a as i128) >> k, "sar {:x} by {}", a, k);
            assert_eq!(jit.call("lt", &args).unwrap(), (a < b) as i64);
            assert_eq!(jit.call("slt", &args).unwrap(), ((a as i128) < (b as i128)) as i64);
            assert_eq!(jit.call("to_i32", &split(a)).unwrap() as i32, a as i32); // x0 holds a 32-bit container
            let x = a as i32;
            let (lo, hi) = jit.call2("from_i32", &[x as i64]).unwrap();
            assert_eq!((lo as u64 as u128) | ((hi as u64 as u128) << 64), x as i128 as u128);
        }
    }
}
