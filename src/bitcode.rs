#![allow(dead_code)]
//! LLVM bitcode, written by hand — the form Apple's GPU compiler takes.
//!
//! A Metal library (`.metallib`) holds LLVM bitcode in Apple's dialect
//! (AIR): typed pointers, address spaces for device (1) and threadgroup
//! (3) memory, `air.*` intrinsics, and metadata that names a kernel's
//! arguments. The driver compiles it for whatever GPU it finds, so it is
//! the portable binary for any Apple machine. Apple's compiler is not
//! in the loop here; what it emits was read with `llvm-bcanalyzer` and
//! `llvm-dis` (tools/probe-air.sh), and this writer produces the same
//! records. Two readers check it: upstream LLVM's disassembler, and the
//! GPU.
//!
//! The bitstream (LLVM's documented container): bits are packed least
//! significant first into 32-bit words; a block opens with abbreviation
//! id 1 (its id, its abbreviation width, and its length in words,
//! patched at the end) and closes with id 0; a record is id 3, its code,
//! its operand count and its operands as 6-bit VBRs. Only two records
//! need a defined abbreviation, because their last operand is a blob:
//! the metadata strings and the string table.
//!
//! A module is built through `Module`: types are interned, values are
//! numbered in LLVM's order — globals, functions, constants, then each
//! function's arguments and instructions — and instruction operands are
//! written relative to the instruction's own number, as the format
//! wants (forward references, phi operands, are signed).

use std::collections::HashMap;

// ---------------------------------------------------------------------------
// the bit writer

pub struct Bits {
    out: Vec<u8>,
    cur: u32,
    ncur: u32,
}

impl Bits {
    pub fn new() -> Bits {
        Bits { out: Vec::new(), cur: 0, ncur: 0 }
    }

    pub fn fixed(&mut self, v: u64, n: u32) {
        let mut v = v;
        let mut n = n;
        while n > 0 {
            let take = (32 - self.ncur).min(n);
            let mask = if take == 32 { u64::MAX } else { (1u64 << take) - 1 };
            self.cur |= ((v & mask) as u32) << self.ncur;
            self.ncur += take;
            v >>= take;
            n -= take;
            if self.ncur == 32 {
                self.out.extend_from_slice(&self.cur.to_le_bytes());
                self.cur = 0;
                self.ncur = 0;
            }
        }
    }

    /// variable bit rate: n-1 bits of value and a continuation bit
    pub fn vbr(&mut self, v: u64, n: u32) {
        let mut v = v;
        let threshold = 1u64 << (n - 1);
        while v >= threshold {
            self.fixed((v & (threshold - 1)) | threshold, n);
            v >>= n - 1;
        }
        self.fixed(v, n);
    }

    /// a signed VBR: the sign in the low bit
    pub fn svbr(&mut self, v: i64, n: u32) {
        let enc = if v >= 0 { (v as u64) << 1 } else { v.unsigned_abs() << 1 | 1 };
        self.vbr(enc, n);
    }

    pub fn align32(&mut self) {
        if self.ncur > 0 {
            self.out.extend_from_slice(&self.cur.to_le_bytes());
            self.cur = 0;
            self.ncur = 0;
        }
    }

    fn word_pos(&self) -> usize {
        self.out.len() / 4
    }

    pub fn finish(mut self) -> Vec<u8> {
        self.align32();
        self.out
    }
}

// ---------------------------------------------------------------------------
// blocks and records

/// the writer with its block stack: (abbreviation width, the word of the
/// length placeholder)
pub struct Stream {
    pub bits: Bits,
    stack: Vec<(u32, usize)>,
    width: u32,
}

impl Stream {
    pub fn new() -> Stream {
        Stream { bits: Bits::new(), stack: Vec::new(), width: 2 }
    }

    pub fn enter(&mut self, block_id: u32, width: u32) {
        self.bits.fixed(1, self.width); // ENTER_SUBBLOCK
        self.bits.vbr(block_id as u64, 8);
        self.bits.vbr(width as u64, 4);
        self.bits.align32();
        let at = self.bits.word_pos();
        self.bits.fixed(0, 32); // length, patched by exit
        self.stack.push((self.width, at));
        self.width = width;
    }

    pub fn exit(&mut self) {
        self.bits.fixed(0, self.width); // END_BLOCK
        self.bits.align32();
        let (width, at) = self.stack.pop().expect("block stack");
        let len = (self.bits.word_pos() - at - 1) as u32;
        self.bits.out[at * 4..at * 4 + 4].copy_from_slice(&len.to_le_bytes());
        self.width = width;
    }

    pub fn record(&mut self, code: u32, ops: &[u64]) {
        self.bits.fixed(3, self.width); // UNABBREV_RECORD
        self.bits.vbr(code as u64, 6);
        self.bits.vbr(ops.len() as u64, 6);
        for &o in ops {
            self.bits.vbr(o, 6);
        }
    }

    /// define a local abbreviation: a literal code, `nvbr` VBR6
    /// operands, then a blob; returns its id (4 upward)
    fn blob_abbrev(&mut self, code: u32, nvbr: usize, next_id: &mut u32) -> u32 {
        self.bits.fixed(2, self.width); // DEFINE_ABBREV
        self.bits.vbr((1 + nvbr + 1) as u64, 5);
        self.bits.fixed(1, 1); // literal
        self.bits.vbr(code as u64, 8);
        for _ in 0..nvbr {
            self.bits.fixed(0, 1);
            self.bits.fixed(2, 3); // VBR
            self.bits.vbr(6, 5);
        }
        self.bits.fixed(0, 1);
        self.bits.fixed(5, 3); // Blob
        let id = *next_id;
        *next_id += 1;
        id
    }

    /// a blob record: [ops...] then the bytes
    fn blob_record(&mut self, abbrev: u32, ops: &[u64], blob: &[u8]) {
        self.bits.fixed(abbrev as u64, self.width);
        for &o in ops {
            self.bits.vbr(o, 6);
        }
        self.bits.vbr(blob.len() as u64, 6);
        self.bits.align32();
        for &b in blob {
            self.bits.fixed(b as u64, 8);
        }
        self.bits.align32();
    }
}

// ---------------------------------------------------------------------------
// the module model

#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub enum Type {
    Void,
    Int(u32),
    Half,
    Float,
    Double,
    Ptr(usize, u32),
    Array(usize, u64),
    /// <n x elem>
    Vector(usize, u64),
    Fn(usize, Vec<usize>),
    Metadata,
    Label,
    /// an anonymous struct: the aggregate a function with several
    /// results returns
    Struct(Vec<usize>),
}

/// a constant, at module level
#[derive(Clone, Debug)]
pub enum Const {
    Int(usize, i64),
    Float(usize, u64),
    Null(usize),
    Undef(usize),
    /// a vector (or array) of constants, by their value ids
    Agg(usize, Vec<usize>),
}

#[derive(Clone, Debug)]
pub enum Md {
    Str(String),
    /// a constant or global value: (type, value id)
    Value(usize, usize),
    /// a node of metadata ids (None = null)
    Node(Vec<Option<usize>>),
}

#[derive(Clone, Debug)]
pub enum Inst {
    /// binop: (opcode, lhs, rhs, flags) → a value
    Bin { op: u32, lhs: usize, rhs: usize, flags: u64 },
    Cast { op: u32, val: usize, ty: usize },
    Gep { elem_ty: usize, base: usize, idx: Vec<usize>, inbounds: bool },
    Load { ptr: usize, ty: usize, align: u32 },
    Store { ptr: usize, val: usize, align: u32 },
    Cmp { pred: u32, lhs: usize, rhs: usize },
    Select { cond: usize, a: usize, b: usize },
    /// call: (function type, callee value id, args) → a value if not void
    Call { fn_ty: usize, callee: usize, args: Vec<usize> },
    Phi { ty: usize, incoming: Vec<(usize, usize)> },
    Alloca { ty: usize, count_val: usize, align: u32 },
    Br { target: usize },
    CondBr { cond: usize, then: usize, els: usize },
    /// switch on an integer: (its type, the value, default block, (case constant id, block)*)
    Switch { ty: usize, val: usize, default: usize, cases: Vec<(usize, usize)> },
    Ret { val: Option<usize> },
    Unreachable,
    /// insert a value into an aggregate at index → the new aggregate
    InsertVal { agg: usize, val: usize, idx: u32 },
    ExtractVal { agg: usize, idx: u32 },
    /// a unary op: fneg is opcode 0
    Unop { op: u32, val: usize },
    /// one lane of a vector
    ExtractElt { vec: usize, idx: usize },
    /// a vector with one lane replaced
    InsertElt { vec: usize, elt: usize, idx: usize },
}

impl Inst {
    fn defines(&self, m: &Module) -> bool {
        match self {
            Inst::Store { .. } | Inst::Br { .. } | Inst::CondBr { .. } | Inst::Switch { .. } | Inst::Ret { .. } | Inst::Unreachable => false,
            Inst::Call { fn_ty, .. } => match &m.types[*fn_ty] {
                Type::Fn(r, _) => !matches!(m.types[*r], Type::Void),
                _ => true,
            },
            _ => true,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Block {
    pub insts: Vec<Inst>,
    /// the builder's id for each instruction (meaningless for one that
    /// defines nothing): pushed in any order, numbered in this one
    pub ids: Vec<usize>,
}

#[derive(Clone, Debug)]
pub struct Function {
    pub name: String,
    pub ty: usize,
    pub is_decl: bool,
    pub blocks: Vec<Block>,
    /// value ids assigned to the arguments, filled by `Module::function`
    pub args: Vec<usize>,
    /// the id of the first instruction value, for numbering
    pub first_inst_value: usize,
    pub nvalues: usize,
    /// LLVM enum attributes on the function (2 alwaysinline, 14 noinline)
    pub attrs: Vec<u64>,
}

#[derive(Clone, Debug)]
pub struct Global {
    pub name: String,
    pub ty: usize,
    pub addrspace: u32,
    pub align: u32,
    /// defined with an `undef` initializer (a threadgroup array), not
    /// declared
    pub undef_init: bool,
}

pub struct Module {
    pub types: Vec<Type>,
    type_ids: HashMap<Type, usize>,
    pub globals: Vec<Global>,
    pub functions: Vec<Function>,
    pub consts: Vec<Const>,
    const_ids: HashMap<(usize, i64, bool), usize>,
    pub metadata: Vec<Md>,
    md_ids: HashMap<String, usize>,
    pub named_md: Vec<(String, Vec<usize>)>,
    pub triple: String,
    pub datalayout: String,
    pub source: String,
    /// value ids: globals then functions get 0.., constants follow
    nglobal_values: usize,
}

impl Module {
    pub fn new() -> Module {
        Module {
            types: Vec::new(),
            type_ids: HashMap::new(),
            globals: Vec::new(),
            functions: Vec::new(),
            consts: Vec::new(),
            const_ids: HashMap::new(),
            metadata: Vec::new(),
            md_ids: HashMap::new(),
            named_md: Vec::new(),
            triple: "air64_v28-apple-macosx26.0.0".into(),
            datalayout: "e-p:64:64:64-i1:8:8-i8:8:8-i16:16:16-i32:32:32-i64:64:64-f32:32:32-f64:64:64-v16:16:16-v24:32:32-v32:32:32-v48:64:64-v64:64:64-v96:128:128-v128:128:128-v192:256:256-v256:256:256-v512:512:512-v1024:1024:1024-n8:16:32".into(),
            source: "probe.ssa".into(),
            nglobal_values: 0,
        }
    }

    pub fn ty(&mut self, t: Type) -> usize {
        if let Some(&i) = self.type_ids.get(&t) {
            return i;
        }
        let i = self.types.len();
        self.types.push(t.clone());
        self.type_ids.insert(t, i);
        i
    }

    pub fn int(&mut self, bits: u32) -> usize {
        self.ty(Type::Int(bits))
    }
    pub fn ptr(&mut self, pointee: usize, addrspace: u32) -> usize {
        self.ty(Type::Ptr(pointee, addrspace))
    }
    pub fn fn_ty(&mut self, ret: usize, params: Vec<usize>) -> usize {
        self.ty(Type::Fn(ret, params))
    }

    /// global values are numbered before constants: declare every global
    /// and function first, then constants, then build bodies
    pub fn global(&mut self, name: &str, elem_ty: usize, addrspace: u32, align: u32) -> usize {
        assert!(self.consts.is_empty(), "declare globals before constants");
        let id = self.globals.len() + self.functions.len();
        self.globals.push(Global { name: name.into(), ty: elem_ty, addrspace, align, undef_init: false });
        self.nglobal_values = self.globals.len() + self.functions.len();
        let _ = id;
        // ids: globals first, then functions — computed at write time
        self.globals.len() - 1
    }

    pub fn function(&mut self, name: &str, ty: usize, is_decl: bool) -> usize {
        assert!(self.consts.is_empty(), "declare functions before constants");
        self.functions.push(Function { name: name.into(), ty, is_decl, blocks: Vec::new(), args: Vec::new(), first_inst_value: 0, nvalues: 0, attrs: Vec::new() });
        self.nglobal_values = self.globals.len() + self.functions.len();
        self.functions.len() - 1
    }

    /// the value id of global `g`
    pub fn global_value(&self, g: usize) -> usize {
        g
    }
    /// the value id of function `f`
    pub fn function_value(&self, f: usize) -> usize {
        self.globals.len() + f
    }

    pub fn const_int(&mut self, ty: usize, v: i64) -> usize {
        let key = (ty, v, false);
        if let Some(&id) = self.const_ids.get(&key) {
            return id;
        }
        let id = self.nglobal_values + self.consts.len();
        self.consts.push(Const::Int(ty, v));
        self.const_ids.insert(key, id);
        id
    }
    pub fn const_float(&mut self, ty: usize, bits: u64) -> usize {
        let key = (ty, bits as i64, true);
        if let Some(&id) = self.const_ids.get(&key) {
            return id;
        }
        let id = self.nglobal_values + self.consts.len();
        self.consts.push(Const::Float(ty, bits));
        self.const_ids.insert(key, id);
        id
    }
    /// a constant vector of the given element constants
    pub fn const_agg(&mut self, ty: usize, elems: Vec<usize>) -> usize {
        let id = self.nglobal_values + self.consts.len();
        self.consts.push(Const::Agg(ty, elems));
        id
    }

    pub fn const_undef(&mut self, ty: usize) -> usize {
        let id = self.nglobal_values + self.consts.len();
        self.consts.push(Const::Undef(ty));
        id
    }

    /// the first value id a function's arguments take
    pub fn first_local_value(&self) -> usize {
        // the undef initializers the writer adds come after the constants
        self.nglobal_values + self.consts.len() + self.globals.iter().filter(|g| g.undef_init).count()
    }

    pub fn md_str(&mut self, s: &str) -> usize {
        if let Some(&i) = self.md_ids.get(s) {
            return i;
        }
        let i = self.metadata.len();
        self.metadata.push(Md::Str(s.into()));
        self.md_ids.insert(s.into(), i);
        i
    }
    pub fn md_value(&mut self, ty: usize, val: usize) -> usize {
        self.metadata.push(Md::Value(ty, val));
        self.metadata.len() - 1
    }
    pub fn md_node(&mut self, items: Vec<Option<usize>>) -> usize {
        self.metadata.push(Md::Node(items));
        self.metadata.len() - 1
    }
    pub fn md_named(&mut self, name: &str, nodes: Vec<usize>) {
        self.named_md.push((name.into(), nodes));
    }

    // ---- writing ----

    fn type_record(&self, t: &Type) -> (u32, Vec<u64>) {
        match t {
            Type::Void => (2, vec![]),
            Type::Float => (3, vec![]),
            Type::Double => (4, vec![]),
            Type::Label => (5, vec![]),
            Type::Int(b) => (7, vec![*b as u64]),
            Type::Ptr(p, a) => (8, vec![*p as u64, *a as u64]),
            Type::Half => (10, vec![]),
            Type::Array(e, n) => (11, vec![*n, *e as u64]),
            Type::Vector(e, n) => (12, vec![*n, *e as u64]),
            Type::Metadata => (16, vec![]),
            Type::Fn(r, ps) => {
                let mut ops = vec![0u64, *r as u64];
                ops.extend(ps.iter().map(|&p| p as u64));
                (21, ops)
            }
            Type::Struct(elts) => {
                let mut ops = vec![0u64]; // not packed
                ops.extend(elts.iter().map(|&e| e as u64));
                (18, ops)
            }
        }
    }

    pub fn write(&self) -> Vec<u8> {
        let mut s = Stream::new();
        // the string table: names of globals and functions
        let mut strtab: Vec<u8> = Vec::new();
        let name_at = |name: &str, strtab: &mut Vec<u8>| -> (u64, u64) {
            let off = strtab.len() as u64;
            strtab.extend_from_slice(name.as_bytes());
            (off, name.len() as u64)
        };

        s.bits.fixed(0x42, 8);
        s.bits.fixed(0x43, 8);
        s.bits.fixed(0x0, 4);
        s.bits.fixed(0xC, 4);
        s.bits.fixed(0xE, 4);
        s.bits.fixed(0xD, 4);

        // IDENTIFICATION
        s.enter(13, 5);
        s.record(1, &[]); // STRING ""
        s.record(2, &[0]); // EPOCH
        s.exit();

        s.enter(8, 3); // MODULE
        s.record(1, &[2]); // VERSION 2
        // attributes: one group per distinct set, on the function
        // (paramidx 0xFFFFFFFF); a list per group; a function names its
        // list by 1-based index
        let mut attr_sets: Vec<Vec<u64>> = Vec::new();
        for f in &self.functions {
            if !f.attrs.is_empty() && !attr_sets.contains(&f.attrs) {
                attr_sets.push(f.attrs.clone());
            }
        }
        if !attr_sets.is_empty() {
            s.enter(10, 3); // PARAMATTR_GROUP
            for (i, set) in attr_sets.iter().enumerate() {
                let mut ops = vec![i as u64 + 1, 0xFFFF_FFFF];
                for a in set {
                    ops.push(0);
                    ops.push(*a);
                }
                s.record(3, &ops);
            }
            s.exit();
            s.enter(9, 3); // PARAMATTR
            for i in 0..attr_sets.len() {
                s.record(2, &[i as u64 + 1]);
            }
            s.exit();
        }
        let attr_index = |f: &Function| -> u64 { attr_sets.iter().position(|a| *a == f.attrs).map_or(0, |i| i as u64 + 1) };

        // types
        s.enter(17, 4);
        s.record(1, &[self.types.len() as u64]); // NUMENTRY
        for t in &self.types {
            let (code, ops) = self.type_record(t);
            s.record(code, &ops);
        }
        s.exit();

        s.record(2, &self.triple.bytes().map(|b| b as u64).collect::<Vec<_>>()); // TRIPLE
        s.record(3, &self.datalayout.bytes().map(|b| b as u64).collect::<Vec<_>>()); // DATALAYOUT
        s.record(16, &self.source.bytes().map(|b| b as u64).collect::<Vec<_>>()); // SOURCE_FILENAME

        // globals: [strtab_offset, strtab_size, pointer type, isconst, initid, linkage, alignment, section, visibility, threadlocal, unnamed_addr, externally_initialized, dllstorageclass, comdat, attributes, preemption]
        let ptr_ty_of = |g: &Global| -> usize {
            *self.type_ids.get(&Type::Ptr(g.ty, g.addrspace)).expect("a global's pointer type must be interned (call Module::ptr for it)")
        };
        // an undef initializer is a constant after all the others; its
        // id is known here
        let mut consts: Vec<Const> = self.consts.clone();
        let mut init_of: Vec<u64> = Vec::new();
        for g in &self.globals {
            if g.undef_init {
                consts.push(Const::Undef(g.ty));
                init_of.push((self.nglobal_values + consts.len()) as u64); // 1-based
            } else {
                init_of.push(0);
            }
        }
        for (gi, g) in self.globals.iter().enumerate() {
            let (off, size) = name_at(&g.name, &mut strtab);
            let pty = ptr_ty_of(g) as u64;
            let align_enc = if g.align == 0 { 0 } else { (g.align.trailing_zeros() + 1) as u64 };
            // explicit type flag (bit 1) set, addrspace in the upper bits
            let ty_and_flags = (g.ty as u64) << 2 | 2 | 0;
            let _ = pty;
            // isconst field also carries "explicit type" (bit 1) and addrspace (bits 2..)
            let isconst = 0u64 | 2 | ((g.addrspace as u64) << 2);
            let _ = ty_and_flags;
            // internal linkage (3), undef initializer: initid 0 = none
            s.record(7, &[off, size, g.ty as u64, isconst, init_of[gi], 3, align_enc, 0, 0, 0, 1, 0, 0, 0, 0, 0]);
        }
        // functions: [strtab_offset, strtab_size, type, callingconv, isproto, linkage, paramattr, alignment, section, visibility, gc, unnamed_addr, prologuedata, dllstorageclass, comdat, prefixdata, personalityfn, preemptionspecifier, addrspace, partition_offset, partition_size]
        for f in &self.functions {
            let (off, size) = name_at(&f.name, &mut strtab);
            s.record(8, &[off, size, f.ty as u64, 0, f.is_decl as u64, 0, attr_index(f), 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        }

        // constants
        if !consts.is_empty() {
            s.enter(11, 4);
            let mut cur_ty = usize::MAX;
            for c in &consts {
                let ty = match c {
                    Const::Int(t, _) | Const::Float(t, _) | Const::Null(t) | Const::Undef(t) | Const::Agg(t, _) => *t,
                };
                if ty != cur_ty {
                    s.record(1, &[ty as u64]); // SETTYPE
                    cur_ty = ty;
                }
                match c {
                    Const::Int(_, v) => {
                        // sign-magnitude, the sign the low bit; i64::MIN's
                        // magnitude is not an i64 (LLVM writes it as 1)
                        let enc = if *v >= 0 { (*v as u64) << 1 } else if *v == i64::MIN { 1 } else { ((-*v) as u64) << 1 | 1 };
                        s.record(4, &[enc]);
                    }
                    Const::Float(_, bits) => s.record(6, &[*bits]),
                    Const::Null(_) => s.record(2, &[]),
                    Const::Undef(_) => s.record(3, &[]),
                    Const::Agg(_, elems) => s.record(7, &elems.iter().map(|&e| e as u64).collect::<Vec<_>>()),
                }
            }
            s.exit();
        }

        // metadata
        if !self.metadata.is_empty() {
            s.enter(15, 3);
            let mut next_abbrev = 4;
            // strings first, as METADATA_STRINGS: [count, offset] blob = lengths (vbr6) then chars
            let strings: Vec<&String> = self.metadata.iter().filter_map(|m| if let Md::Str(s) = m { Some(s) } else { None }).collect();
            // the metadata ids as LLVM numbers them: strings get 0..n, then the rest in order
            let mut md_index: Vec<usize> = vec![0; self.metadata.len()];
            let mut k = 0;
            for (i, m) in self.metadata.iter().enumerate() {
                if let Md::Str(_) = m {
                    md_index[i] = k;
                    k += 1;
                }
            }
            for (i, m) in self.metadata.iter().enumerate() {
                if !matches!(m, Md::Str(_)) {
                    md_index[i] = k;
                    k += 1;
                }
            }
            if !strings.is_empty() {
                let abbrev = s.blob_abbrev(35, 2, &mut next_abbrev);
                let mut lens = Bits::new();
                for st in &strings {
                    lens.vbr(st.len() as u64, 6);
                }
                let lens = lens.finish();
                let mut blob = lens.clone();
                for st in &strings {
                    blob.extend_from_slice(st.as_bytes());
                }
                s.blob_record(abbrev, &[strings.len() as u64, lens.len() as u64], &blob);
            }
            for m in &self.metadata {
                match m {
                    Md::Str(_) => {}
                    Md::Value(ty, val) => s.record(2, &[*ty as u64, *val as u64]),
                    Md::Node(items) => {
                        let ops: Vec<u64> = items.iter().map(|it| it.map(|i| md_index[i] as u64 + 1).unwrap_or(0)).collect();
                        s.record(3, &ops);
                    }
                }
            }
            for (name, nodes) in &self.named_md {
                s.record(4, &name.bytes().map(|b| b as u64).collect::<Vec<_>>()); // NAME
                s.record(10, &nodes.iter().map(|&n| md_index[n] as u64).collect::<Vec<_>>()); // NAMED_NODE
            }
            s.exit();
        }

        // function bodies
        for f in &self.functions {
            if f.is_decl {
                continue;
            }
            self.write_function(&mut s, f);
        }

        s.exit(); // MODULE

        // the string table: a top-level block after the module
        s.enter(23, 3);
        let mut next_abbrev = 4;
        let abbrev = s.blob_abbrev(1, 0, &mut next_abbrev);
        s.blob_record(abbrev, &[], &strtab);
        s.exit();

        let body = s.bits.finish();
        // the wrapper header Apple's tools put in front
        let mut out = Vec::new();
        out.extend_from_slice(&0x0B17C0DEu32.to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(&20u32.to_le_bytes());
        out.extend_from_slice(&(body.len() as u32).to_le_bytes());
        out.extend_from_slice(&0xFFFFFFFFu32.to_le_bytes());
        out.extend_from_slice(&body);
        out
    }

    fn write_function(&self, s: &mut Stream, f: &Function) {
        s.enter(12, 4);
        s.record(1, &[f.blocks.len() as u64]); // DECLAREBLOCKS
        // value numbering: arguments first, then instructions that define
        let mut next = self.first_local_value();
        let nargs = match &self.types[f.ty] {
            Type::Fn(_, ps) => ps.len(),
            _ => 0,
        };
        assert_eq!(f.args.len(), nargs);
        next += nargs;
        // instruction value ids in the order written, which need not be
        // the order the builder handed them out (blocks are filled in
        // any order): the builder's id maps to the written one
        let mut inst_ids: Vec<Vec<Option<usize>>> = Vec::new();
        let base = self.first_local_value();
        let mut placed: HashMap<usize, usize> = (0..nargs).map(|i| (LOCAL + i, base + i)).collect();
        {
            let mut n = next;
            for b in &f.blocks {
                let mut ids = Vec::new();
                for (inst, &id) in b.insts.iter().zip(&b.ids) {
                    if inst.defines(self) {
                        ids.push(Some(n));
                        placed.insert(id, n);
                        n += 1;
                    } else {
                        ids.push(None);
                    }
                }
                inst_ids.push(ids);
            }
        }
        let mut cur = next;
        // a local id is placed after the constants; others are absolute
        let abs = |v: usize| if v >= LOCAL { *placed.get(&v).unwrap_or_else(|| panic!("value {} used but never defined", v - LOCAL)) } else { v };
        let rel = |cur: usize, v: usize| (cur as i64 - abs(v) as i64) as u64;
        for (bi, b) in f.blocks.iter().enumerate() {
            for (ii, inst) in b.insts.iter().enumerate() {
                match inst {
                    Inst::Bin { op, lhs, rhs, flags } => {
                        let mut ops = vec![rel(cur, *lhs), rel(cur, *rhs), *op as u64];
                        if *flags != 0 {
                            ops.push(*flags);
                        }
                        s.record(2, &ops);
                    }
                    Inst::Cast { op, val, ty } => s.record(3, &[rel(cur, *val), *ty as u64, *op as u64]),
                    Inst::Gep { elem_ty, base, idx, inbounds } => {
                        let mut ops = vec![*inbounds as u64, *elem_ty as u64, rel(cur, *base)];
                        ops.extend(idx.iter().map(|&i| rel(cur, i)));
                        s.record(43, &ops);
                    }
                    Inst::Load { ptr, ty, align } => s.record(20, &[rel(cur, *ptr), *ty as u64, (align.trailing_zeros() + 1) as u64, 0]),
                    Inst::Store { ptr, val, align } => s.record(44, &[rel(cur, *ptr), rel(cur, *val), (align.trailing_zeros() + 1) as u64, 0]),
                    Inst::Cmp { pred, lhs, rhs } => s.record(28, &[rel(cur, *lhs), rel(cur, *rhs), *pred as u64]),
                    Inst::Select { cond, a, b } => s.record(29, &[rel(cur, *a), rel(cur, *b), rel(cur, *cond)]),
                    Inst::Call { fn_ty, callee, args } => {
                        // [paramattrs, cc | explicit-type bit 15, fnty, callee, args...]
                        let mut ops = vec![0u64, 1 << 15, *fn_ty as u64, rel(cur, *callee)];
                        ops.extend(args.iter().map(|&a| rel(cur, a)));
                        s.record(34, &ops);
                    }
                    Inst::Phi { ty, incoming } => {
                        let mut ops = vec![*ty as u64];
                        for (v, blk) in incoming {
                            let d = cur as i64 - abs(*v) as i64;
                            let enc = if d >= 0 { (d as u64) << 1 } else { ((-d) as u64) << 1 | 1 };
                            ops.push(enc);
                            ops.push(*blk as u64);
                        }
                        s.record(16, &ops);
                    }
                    Inst::Alloca { ty, count_val, align } => {
                        // [instty, opty, op, align] — op is an absolute value id
                        let opty = match &self.consts[abs(*count_val) - self.nglobal_values] {
                            Const::Int(t, _) => *t,
                            _ => panic!("alloca count must be an int constant"),
                        };
                        let align_enc = (align.trailing_zeros() + 1) as u64 | (1 << 6); // bit 6: explicit type
                        s.record(19, &[*ty as u64, opty as u64, abs(*count_val) as u64, align_enc]);
                    }
                    Inst::Br { target } => s.record(11, &[*target as u64]),
                    Inst::CondBr { cond, then, els } => s.record(11, &[*then as u64, *els as u64, rel(cur, *cond)]),
                    Inst::Switch { ty, val, default, cases } => {
                        // [opty, cond, default, (case value id — absolute, block)*]
                        let mut ops = vec![*ty as u64, rel(cur, *val), *default as u64];
                        for (c, b) in cases {
                            ops.push(abs(*c) as u64);
                            ops.push(*b as u64);
                        }
                        s.record(12, &ops);
                    }
                    Inst::InsertVal { agg, val, idx } => s.record(27, &[rel(cur, *agg), rel(cur, *val), *idx as u64]),
                    Inst::ExtractVal { agg, idx } => s.record(26, &[rel(cur, *agg), *idx as u64]),
                    Inst::ExtractElt { vec, idx } => s.record(6, &[rel(cur, *vec), rel(cur, *idx)]),
                    Inst::InsertElt { vec, elt, idx } => s.record(7, &[rel(cur, *vec), rel(cur, *elt), rel(cur, *idx)]),
                    Inst::Unop { op, val } => s.record(56, &[rel(cur, *val), *op as u64]),
                    Inst::Ret { val: Some(v) } => s.record(10, &[rel(cur, *v)]),
                    Inst::Ret { val: None } => s.record(10, &[]),
                    Inst::Unreachable => s.record(15, &[]),
                }
                if inst_ids[bi][ii].is_some() {
                    cur += 1;
                }
            }
        }
        s.exit();
    }

    /// the value id an instruction will define, given the block/instruction
    /// position — for building bodies incrementally: see `FnBuilder`
    pub fn nglobal_values(&self) -> usize {
        self.nglobal_values
    }
}

/// function-local values (arguments, instructions) are numbered in a
/// space of their own while a body is built — constants may still be
/// added — and placed after the constants when the module is written
pub const LOCAL: usize = 1 << 40;

/// builds a function body with value ids handed out in order
pub struct FnBuilder {
    pub f: usize,
    next: usize,
    pub blocks: Vec<Block>,
    cur: usize,
    /// blocks in the order first entered: the order they are written,
    /// so that a block split off another (its rest moved to a new one)
    /// comes before the blocks that use what it defines
    order: Vec<usize>,
}

impl FnBuilder {
    pub fn new(m: &mut Module, f: usize) -> FnBuilder {
        let nargs = match &m.types[m.functions[f].ty] {
            Type::Fn(_, ps) => ps.len(),
            _ => 0,
        };
        m.functions[f].args = (LOCAL..LOCAL + nargs).collect();
        FnBuilder { f, next: LOCAL + nargs, blocks: vec![Block { insts: Vec::new(), ids: Vec::new() }], cur: 0, order: vec![0] }
    }

    pub fn arg(&self, m: &Module, i: usize) -> usize {
        m.functions[self.f].args[i]
    }

    pub fn block(&mut self) -> usize {
        self.blocks.push(Block { insts: Vec::new(), ids: Vec::new() });
        self.blocks.len() - 1
    }

    pub fn at(&mut self, b: usize) {
        self.cur = b;
        if !self.order.contains(&b) {
            self.order.push(b);
        }
    }

    /// append an instruction; returns the value it defines (or the id it
    /// would have — meaningless for non-defining ones)
    pub fn push(&mut self, m: &Module, inst: Inst) -> usize {
        let defines = inst.defines(m);
        self.blocks[self.cur].insts.push(inst);
        self.blocks[self.cur].ids.push(self.next);
        if defines {
            self.next += 1;
            self.next - 1
        } else {
            self.next
        }
    }

    /// append to a block without entering it (a phi placed ahead of time)
    pub fn push_in(&mut self, m: &Module, b: usize, inst: Inst) -> usize {
        let was = self.cur;
        self.cur = b;
        let id = self.push(m, inst);
        self.cur = was;
        id
    }

    /// the id the next defining instruction gets (for phis referring forward)
    pub fn peek(&self) -> usize {
        self.next
    }

    pub fn finish(mut self, m: &mut Module) {
        // blocks in entry order, never-entered ones last; every block
        // index in an instruction renumbered to match
        let mut order = self.order.clone();
        for b in 0..self.blocks.len() {
            if !order.contains(&b) {
                order.push(b);
            }
        }
        let mut new_of = vec![0usize; self.blocks.len()];
        for (k, &b) in order.iter().enumerate() {
            new_of[b] = k;
        }
        for blk in &mut self.blocks {
            for inst in &mut blk.insts {
                match inst {
                    Inst::Br { target } => *target = new_of[*target],
                    Inst::CondBr { then, els, .. } => {
                        *then = new_of[*then];
                        *els = new_of[*els];
                    }
                    Inst::Switch { default, cases, .. } => {
                        *default = new_of[*default];
                        for c in cases.iter_mut() {
                            c.1 = new_of[c.1];
                        }
                    }
                    Inst::Phi { incoming, .. } => {
                        for i in incoming.iter_mut() {
                            i.1 = new_of[i.1];
                        }
                    }
                    _ => {}
                }
            }
        }
        let mut blocks: Vec<Option<Block>> = self.blocks.into_iter().map(Some).collect();
        m.functions[self.f].blocks = order.iter().map(|&b| blocks[b].take().unwrap()).collect();
    }
}

// ---------------------------------------------------------------------------
// the .metallib container

/// A Metal library holding one or more compute kernels, all in one
/// bitcode module: the header, a tag list per function, two empty
/// metadata sections, and the bitcode — as Apple's `metallib` lays it out
pub fn metallib(bitcode: &[u8], kernels: &[&str]) -> Vec<u8> {
    fn tag(t: &[u8; 4], v: &[u8]) -> Vec<u8> {
        let mut o = t.to_vec();
        o.extend_from_slice(&(v.len() as u16).to_le_bytes());
        o.extend_from_slice(v);
        o
    }
    let hash = sha256(bitcode);
    let mut funclist = (kernels.len() as u32).to_le_bytes().to_vec();
    for k in kernels {
        let mut name = k.as_bytes().to_vec();
        name.push(0);
        let mut entry = Vec::new();
        entry.extend(tag(b"NAME", &name));
        entry.extend(tag(b"TYPE", &[2]));
        entry.extend(tag(b"HASH", &hash));
        entry.extend(tag(b"MDSZ", &(bitcode.len() as u64).to_le_bytes()));
        let mut offt = Vec::new();
        offt.extend_from_slice(&0u64.to_le_bytes());
        offt.extend_from_slice(&0u64.to_le_bytes());
        offt.extend_from_slice(&0u64.to_le_bytes());
        entry.extend(tag(b"OFFT", &offt));
        let mut vers = Vec::new();
        for v in [2u16, 8, 4, 0] {
            vers.extend_from_slice(&v.to_le_bytes());
        }
        entry.extend(tag(b"VERS", &vers));
        entry.extend_from_slice(b"ENDT");
        funclist.extend_from_slice(&(entry.len() as u32).to_le_bytes());
        funclist.extend(entry);
    }
    let meta = {
        let mut m = 4u32.to_le_bytes().to_vec();
        m.extend_from_slice(b"ENDT");
        m
    };
    let hdr_len = 0x58u64;
    let fl_off = hdr_len;
    let pub_off = fl_off + funclist.len() as u64;
    let prv_off = pub_off + meta.len() as u64;
    let bc_off = prv_off + meta.len() as u64;
    let total = bc_off + bitcode.len() as u64;
    let mut out = Vec::new();
    out.extend_from_slice(b"MTLB");
    out.extend_from_slice(&0x8001u16.to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());
    out.extend_from_slice(&9u16.to_le_bytes());
    out.push(0);
    out.push(129);
    out.extend_from_slice(&26u16.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&total.to_le_bytes());
    for (off, len) in [(fl_off, funclist.len() as u64), (pub_off, meta.len() as u64), (prv_off, meta.len() as u64), (bc_off, bitcode.len() as u64)] {
        out.extend_from_slice(&off.to_le_bytes());
        out.extend_from_slice(&len.to_le_bytes());
    }
    assert_eq!(out.len() as u64, hdr_len);
    out.extend(funclist);
    out.extend_from_slice(&meta);
    out.extend_from_slice(&meta);
    out.extend_from_slice(bitcode);
    out
}

/// SHA-256, as the container's HASH tag wants it
pub fn sha256(data: &[u8]) -> [u8; 32] {
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786,
        0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13,
        0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a,
        0x5b9cca4f, 0x682e6ff3, 0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
    ];
    let mut h: [u32; 8] = [0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19];
    let mut msg = data.to_vec();
    let bitlen = (data.len() as u64) * 8;
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&bitlen.to_be_bytes());
    for chunk in msg.chunks(64) {
        let mut w = [0u32; 64];
        for i in 0..16 {
            w[i] = u32::from_be_bytes([chunk[4 * i], chunk[4 * i + 1], chunk[4 * i + 2], chunk[4 * i + 3]]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16].wrapping_add(s0).wrapping_add(w[i - 7]).wrapping_add(s1);
        }
        let mut a = h;
        for i in 0..64 {
            let s1 = a[4].rotate_right(6) ^ a[4].rotate_right(11) ^ a[4].rotate_right(25);
            let ch = (a[4] & a[5]) ^ (!a[4] & a[6]);
            let t1 = a[7].wrapping_add(s1).wrapping_add(ch).wrapping_add(K[i]).wrapping_add(w[i]);
            let s0 = a[0].rotate_right(2) ^ a[0].rotate_right(13) ^ a[0].rotate_right(22);
            let maj = (a[0] & a[1]) ^ (a[0] & a[2]) ^ (a[1] & a[2]);
            let t2 = s0.wrapping_add(maj);
            a = [t1.wrapping_add(t2), a[0], a[1], a[2], a[3].wrapping_add(t1), a[4], a[5], a[6]];
        }
        for i in 0..8 {
            h[i] = h[i].wrapping_add(a[i]);
        }
    }
    let mut out = [0u8; 32];
    for i in 0..8 {
        out[4 * i..4 * i + 4].copy_from_slice(&h[i].to_be_bytes());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha256_known() {
        let h = sha256(b"abc");
        assert_eq!(h[..4], [0xba, 0x78, 0x16, 0xbf]);
    }

    /// the add1 kernel, by hand: buf[tid] = buf[tid] + 1
    pub fn add1_module() -> (Module, &'static str) {
        let mut m = Module::new();
        let void = m.ty(Type::Void);
        let i32t = m.int(32);
        let i64t = m.int(64);
        let p1_i32 = m.ptr(i32t, 1);
        let fty = m.fn_ty(void, vec![p1_i32, i32t]);
        let metadata = m.ty(Type::Metadata);
        let _ = metadata;
        let f = m.function("add1", fty, false);
        // constants used by the body and the metadata
        let one = m.const_int(i32t, 1);
        let c0 = m.const_int(i32t, 0);
        let c1 = m.const_int(i32t, 1);
        let c4 = m.const_int(i32t, 4);
        let _ = c1;
        // the body
        let mut b = FnBuilder::new(&mut m, f);
        let buf = b.arg(&m, 0);
        let tid = b.arg(&m, 1);
        let idx = b.push(&m, Inst::Cast { op: 1, val: tid, ty: i64t }); // zext
        let p = b.push(&m, Inst::Gep { elem_ty: i32t, base: buf, idx: vec![idx], inbounds: true });
        let v = b.push(&m, Inst::Load { ptr: p, ty: i32t, align: 4 });
        let w = b.push(&m, Inst::Bin { op: 0, lhs: v, rhs: one, flags: 0 }); // add
        b.push(&m, Inst::Store { ptr: p, val: w, align: 4 });
        b.push(&m, Inst::Ret { val: None });
        b.finish(&mut m);
        // the kernel's metadata, in the shape Apple's compiler emits
        let fv = m.function_value(f);
        let ptr_fty = m.ptr(fty, 0);
        let fn_md = m.md_value(ptr_fty, fv);
        let empty = m.md_node(vec![]);
        let s = |m: &mut Module, t: &str| Some(m.md_str(t));
        let v0 = m.md_value(i32t, c0);
        let v1 = m.md_value(i32t, c1);
        let v4 = m.md_value(i32t, c4);
        let arg0 = {
            let items = vec![
                Some(v0), s(&mut m, "air.buffer"), s(&mut m, "air.location_index"), Some(v0), Some(v1), s(&mut m, "air.read_write"),
                s(&mut m, "air.address_space"), Some(v1), s(&mut m, "air.arg_type_size"), Some(v4), s(&mut m, "air.arg_type_align_size"), Some(v4),
                s(&mut m, "air.arg_type_name"), s(&mut m, "int"), s(&mut m, "air.arg_name"), s(&mut m, "buf"),
            ];
            m.md_node(items)
        };
        let arg1 = {
            let items = vec![Some(v1), s(&mut m, "air.thread_position_in_grid"), s(&mut m, "air.arg_type_name"), s(&mut m, "uint"), s(&mut m, "air.arg_name"), s(&mut m, "tid")];
            m.md_node(items)
        };
        let args = m.md_node(vec![Some(arg0), Some(arg1)]);
        let kernel = m.md_node(vec![Some(fn_md), Some(empty), Some(args)]);
        m.md_named("air.kernel", vec![kernel]);
        let c2 = m.const_int(i32t, 2);
        let c8 = m.const_int(i32t, 8);
        let v2 = m.md_value(i32t, c2);
        let v8 = m.md_value(i32t, c8);
        let version = m.md_node(vec![Some(v2), Some(v8), Some(v0)]);
        m.md_named("air.version", vec![version]);
        let metal = m.md_str("Metal");
        let lang = m.md_node(vec![Some(metal), Some(v4), Some(v0), Some(v0)]);
        m.md_named("air.language_version", vec![lang]);
        (m, "add1")
    }

    /// bisecting: the same function with no metadata at all
    #[test]
    fn writes_a_bare_function() {
        let mut m = Module::new();
        let void = m.ty(Type::Void);
        let i32t = m.int(32);
        let i64t = m.int(64);
        let p1_i32 = m.ptr(i32t, 1);
        let fty = m.fn_ty(void, vec![p1_i32, i32t]);
        let f = m.function("add1", fty, false);
        let one = m.const_int(i32t, 1);
        let mut b = FnBuilder::new(&mut m, f);
        let buf = b.arg(&m, 0);
        let tid = b.arg(&m, 1);
        let idx = b.push(&m, Inst::Cast { op: 1, val: tid, ty: i64t });
        let p = b.push(&m, Inst::Gep { elem_ty: i32t, base: buf, idx: vec![idx], inbounds: true });
        let v = b.push(&m, Inst::Load { ptr: p, ty: i32t, align: 4 });
        let w = b.push(&m, Inst::Bin { op: 0, lhs: v, rhs: one, flags: 0 });
        b.push(&m, Inst::Store { ptr: p, val: w, align: 4 });
        b.push(&m, Inst::Ret { val: None });
        b.finish(&mut m);
        let dir = std::env::temp_dir().join("probe-air");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("bare.bc");
        std::fs::write(&path, m.write()).unwrap();
        let dis = std::process::Command::new("/opt/homebrew/opt/llvm/bin/llvm-dis").arg(&path).arg("-o").arg("-").output().expect("llvm-dis");
        assert!(dis.status.success(), "llvm-dis: {}", String::from_utf8_lossy(&dis.stderr));
    }

    /// bisecting: a module with one function that only returns
    #[test]
    fn writes_a_trivial_function() {
        let mut m = Module::new();
        let void = m.ty(Type::Void);
        let fty = m.fn_ty(void, vec![]);
        let f = m.function("nop", fty, false);
        let mut b = FnBuilder::new(&mut m, f);
        b.push(&m, Inst::Ret { val: None });
        b.finish(&mut m);
        let dir = std::env::temp_dir().join("probe-air");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("nop.bc");
        std::fs::write(&path, m.write()).unwrap();
        let dis = std::process::Command::new("/opt/homebrew/opt/llvm/bin/llvm-dis").arg(&path).arg("-o").arg("-").output().expect("llvm-dis");
        assert!(dis.status.success(), "llvm-dis: {}", String::from_utf8_lossy(&dis.stderr));
    }

    #[test]
    fn writes_a_module_that_llvm_reads() {
        let (m, _) = add1_module();
        let bc = m.write();
        let dir = std::env::temp_dir().join("probe-air");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("add1.bc");
        std::fs::write(&path, &bc).unwrap();
        std::fs::write(dir.join("add1.metallib"), metallib(&bc, &["add1"])).unwrap();
        let dis = std::process::Command::new("/opt/homebrew/opt/llvm/bin/llvm-dis").arg(&path).arg("-o").arg("-").output().expect("llvm-dis");
        let text = String::from_utf8_lossy(&dis.stdout).to_string();
        assert!(dis.status.success(), "llvm-dis: {}", String::from_utf8_lossy(&dis.stderr));
        assert!(text.contains("define void @add1(ptr addrspace(1)"), "{}", text);
        assert!(text.contains("!air.kernel"), "{}", text);
    }
}
