//! AIR: the SSA as LLVM bitcode in Apple's GPU dialect, in a `.metallib`
//! the driver compiles for whatever GPU it finds — the binary for any
//! Apple machine, produced with none of Apple's tools (src/bitcode.rs
//! writes the bytes; tools/probe-air.sh is how the dialect was read off
//! their compiler).
//!
//! LLVM's IR is the nearest neighbour ours has: a basic block is a
//! basic block, a block parameter is a phi, values are values. What it
//! does not have, and how it is given:
//!
//! - one memory. A `ptr` is an i64 offset into the program's memory,
//!   as on wasm: every function takes two hidden parameters, `env` (the
//!   memory, `i8 addrspace(1)*`, data at offset 0) and `slab` (the
//!   offset of this thread's scratch, from which a callee gets the part
//!   past the caller's own), and an access is a GEP from `env` cast to
//!   the type loaded or stored. Pointers in memory are their offsets,
//!   a pointer cast to an integer is the offset, and an address in a
//!   driver is an offset it chose.
//! - several results: a function returns a struct, the call takes it
//!   apart.
//! - function values: an index into the address-taken functions, and
//!   a per-signature dispatcher that switches on it.
//! - recursion: Metal refuses it, so a function that can reach itself,
//!   and everything that calls it, is left out and reported.
//! - integer widths: Apple's back end takes i8/i16/i32/i64 and not
//!   much else (an `or` with a wide constant on an i57 crashed it), so
//!   every integer lives in the container of its width, normalized —
//!   zero-extended if unsigned, sign-extended if signed, as wasm keeps
//!   them — with arithmetic renormalizing; memory sizes match the
//!   containers already.
//! - threadgroups: `fn __kernel(mem, area, id, lane, group)` also gets
//!   its place in its group and its group's number; `lib/gpu.ssa`'s
//!   `group_load`/`group_store`/`group_sync` are platform rules here —
//!   a 16 KB `addrspace(3)` array and `air.wg.barrier` — and plain
//!   functions over data everywhere else.
//! - vectors: `targets/air.platform` says `builtin vectors`, so a `TxN`
//!   reaches here whole, as `<N x T>`: arithmetic, comparisons and
//!   conversions are LLVM's on vectors, `pack`/`unpack`/`get`/`set` are
//!   insertelement/extractelement, a rule applies to the vector (fadd
//!   on `<4 x float>`, `air.sqrt.v4f32`), and a library call on lanes
//!   is made per lane.
//! - inlining: every function is `alwaysinline`, so a kernel is one
//!   body (see the note at the declarations for why that is also the
//!   correct one).
//! - floats: values are their bits, as on wasm; a platform rule names
//!   the operation (`fadd`, `air.sqrt.f32`) and the emitter bitcasts
//!   around it. Nothing is `fast`.
//! - the entry: `fn __kernel(mem: ptr, area: ptr, id: i64)` in the
//!   program becomes a kernel taking two buffers and the thread's
//!   position in the grid, with the metadata that names them.

use crate::bitcode::{self, FnBuilder, Inst as B, Module as BModule, Type as BType};
use crate::platform::{Natives, Platform};
use crate::ssa::{BinOp, CastOp, Cond, Function, Inst, Module, Type, ValueId};
use std::collections::HashMap;

/// what the driver needs to lay out the program's memory
pub struct Layout {
    /// the data image, at offset 0 of the memory buffer
    pub data: Vec<u8>,
    /// bytes of scratch each thread needs, after the data
    pub slab: u64,
}

pub struct Compiled {
    pub bitcode: Vec<u8>,
    pub metallib: Vec<u8>,
    pub layout: Layout,
    /// functions left out, with why
    pub skipped: Vec<(String, String)>,
    pub has_kernel: bool,
}

// LLVM's numbers
const OP_ADD: u32 = 0;
const OP_SUB: u32 = 1;
const OP_MUL: u32 = 2;
const OP_UDIV: u32 = 3;
const OP_SDIV: u32 = 4;
const OP_UREM: u32 = 5;
const OP_SREM: u32 = 6;
const OP_SHL: u32 = 7;
const OP_LSHR: u32 = 8;
const OP_ASHR: u32 = 9;
const OP_AND: u32 = 10;
const OP_OR: u32 = 11;
const OP_XOR: u32 = 12;
const CAST_TRUNC: u32 = 0;
const CAST_ZEXT: u32 = 1;
const CAST_SEXT: u32 = 2;
const CAST_BITCAST: u32 = 11;
/// LLVM's attribute kinds
const ATTR_CONVERGENT: u64 = 43;
/// threadgroup memory a program may use, in bytes
const GROUP_BYTES: u64 = 16384;

/// which functions AIR cannot have — a function that can reach itself,
/// and everything that can reach one — with why, given the
/// address-taken functions (an indirect call may reach any of them
/// with its signature)
pub fn left_out(module: &Module, taken: &[String]) -> (Vec<bool>, Vec<(String, String)>) {
    let edges = call_edges(module, taken, None);
    let reaches = |from: usize, to: usize| -> bool {
        let mut seen = vec![false; edges.len()];
        let mut stack = edges[from].clone();
        while let Some(n) = stack.pop() {
            if n == to {
                return true;
            }
            if !seen[n] {
                seen[n] = true;
                stack.extend(edges[n].iter().copied());
            }
        }
        false
    };
    let recursive: Vec<bool> = (0..module.funcs.len()).map(|i| reaches(i, i)).collect();
    let mut out = vec![false; module.funcs.len()];
    let mut skipped = Vec::new();
    for i in 0..module.funcs.len() {
        if recursive[i] {
            out[i] = true;
            skipped.push((module.funcs[i].name.clone(), "recursion: Metal has no call stack".to_string()));
        } else if (0..module.funcs.len()).any(|j| recursive[j] && reaches(i, j)) {
            out[i] = true;
            skipped.push((module.funcs[i].name.clone(), "calls a recursive function".to_string()));
        }
    }
    (out, skipped)
}

/// what the kernel cannot reach: Apple's compiler takes every function
/// in a module, so the rest — a library's unused parts — is left out too
fn unreachable_from_kernel(module: &Module, taken: &[String], natives: &Natives) -> Vec<bool> {
    // a call a platform rule takes over reaches no library function
    let edges = call_edges(module, taken, Some(natives));
    let mut seen = vec![false; module.funcs.len()];
    if !module.funcs.iter().any(|f| f.name == "__kernel") {
        return seen; // no kernel: everything is kept, for looking at
    }
    let mut stack: Vec<usize> = module.funcs.iter().enumerate().filter(|(_, f)| f.name == "__kernel" || taken.contains(&f.name)).map(|(i, _)| i).collect();
    while let Some(n) = stack.pop() {
        if !seen[n] {
            seen[n] = true;
            stack.extend(edges[n].iter().copied());
        }
    }
    seen.iter().map(|&s| !s).collect()
}

/// the call graph, with an indirect call an edge to every
/// address-taken function of its signature
fn call_edges(module: &Module, taken: &[String], natives: Option<&Natives>) -> Vec<Vec<usize>> {
    let sig_of = |f: &Function| -> (Vec<Type>, Vec<Type>) { (f.params.iter().map(|&p| f.ty(p)).collect(), f.rets.clone()) };
    let names: HashMap<&str, usize> = module.funcs.iter().enumerate().map(|(i, f)| (f.name.as_str(), i)).collect();
    let mut edges: Vec<Vec<usize>> = vec![Vec::new(); module.funcs.len()];
    for (i, f) in module.funcs.iter().enumerate() {
        for inst in f.blocks.iter().flat_map(|b| &b.insts) {
            match inst {
                Inst::Call { callee, .. } => {
                    if natives.is_some_and(|n| n.get(callee).is_some()) {
                        continue;
                    }
                    if let Some(&j) = names.get(callee.as_str()) {
                        edges[i].push(j);
                    }
                }
                Inst::CallInd { callee, .. } => {
                    let sig = f.sig(f.ty(*callee)).cloned();
                    for t in taken {
                        let tf = module.func(t).unwrap();
                        if Some(sig_of(tf)) == sig {
                            edges[i].push(names[t.as_str()]);
                        }
                    }
                }
                _ => {}
            }
        }
    }
    edges
}

/// the functions a program's AIR leaves out, by name
pub fn skipped_names(module: &Module) -> Vec<String> {
    let taken = address_taken(module);
    left_out(module, &taken).1.into_iter().map(|(n, _)| n).collect()
}

fn address_taken(module: &Module) -> Vec<String> {
    let mut taken: Vec<String> = Vec::new();
    for f in &module.funcs {
        for inst in f.blocks.iter().flat_map(|b| &b.insts) {
            if let Inst::FnAddr { name, .. } = inst {
                if !taken.contains(name) {
                    taken.push(name.clone());
                }
            }
        }
    }
    taken
}

pub fn compile_with(module: &Module, platform: &Platform) -> Result<Compiled, String> {
    let natives = platform.natives(module);
    let (data, data_offsets) = crate::ssa::layout_data(module);
    let data_size = ((data.len() as u64) + 15) & !15;

    // address-taken functions, in order of first appearance: a function
    // value is its index here
    let taken = address_taken(module);
    let (mut out, skipped) = left_out(module, &taken);
    for (i, u) in unreachable_from_kernel(module, &taken, &natives).into_iter().enumerate() {
        out[i] |= u;
    }

    // per-thread scratch: any call chain's frames fit in the sum of all
    let scratch_of: Vec<i64> = module.funcs.iter().map(|f| crate::emit::scratch_layout(f, 0).1).collect();
    let slab: u64 = scratch_of.iter().sum::<i64>() as u64 + 16;

    let mut m = BModule::new();
    let void = m.ty(BType::Void);
    let i8t = m.int(8);
    let i32t = m.int(32);
    let i64t = m.int(64);
    let ptr_ty = m.ptr(i8t, 1);
    let mut cx = Cx {
        m,
        natives: &natives,
        module,
        ptr_ty,
        void,
        i32t,
        i64t,
        fn_ids: HashMap::new(),
        decls: HashMap::new(),
        taken: taken.clone(),
        data_offsets,
        scratch_of: scratch_of.clone(),
        dispatchers: HashMap::new(),
        group: None,
        kernel_arity: 3,
    };
    // threadgroup memory, before any constant (globals come first)
    let uses_group = module.funcs.iter().enumerate().filter(|(i, _)| !out[*i]).flat_map(|(_, f)| f.blocks.iter().flat_map(|b| &b.insts)).any(|inst| matches!(inst, Inst::Call { callee, .. } if natives.get(callee).is_some_and(|n| n.rule.lines[0].template.as_deref().is_some_and(|k| k.starts_with("air.group")))));
    if uses_group {
        let arr = cx.m.ty(BType::Array(i8t, GROUP_BYTES));
        cx.m.ptr(arr, 3);
        let g = cx.m.global("__group", arr, 3, 16);
        cx.m.globals[g].undef_init = true;
        cx.group = Some(g);
    }

    // every global value first: functions, dispatchers, the kernel,
    // the intrinsics rules use
    for (i, f) in module.funcs.iter().enumerate() {
        if out[i] {
            continue;
        }
        let fty = cx.fn_type(f);
        let name = if f.name == "__kernel" { "__kernel_body".to_string() } else { f.name.clone() };
        let id = cx.m.function(&name, fty, false);
        // every function `alwaysinline`: the kernel becomes one body,
        // which is what a GPU wants — and Apple's optimizer, left to
        // choose, miscompiled a 128-bit division whose callees had three
        // callers (upstream LLVM ran the same bitcode right; noinline
        // was right too, and six times slower). PROBE_AIR_INLINE=
        // none|noinline puts that back, for looking
        match std::env::var("PROBE_AIR_INLINE").as_deref() {
            Ok("noinline") => cx.m.functions[id].attrs.push(14),
            Ok("none") => {}
            _ => cx.m.functions[id].attrs.push(2),
        }
        cx.fn_ids.insert(f.name.clone(), (id, fty));
    }
    let mut sigs: Vec<(Vec<Type>, Vec<Type>)> = Vec::new();
    for (i, f) in module.funcs.iter().enumerate() {
        if out[i] {
            continue;
        }
        for inst in f.blocks.iter().flat_map(|b| &b.insts) {
            if let Inst::CallInd { callee, .. } = inst {
                let sig = f.sig(f.ty(*callee)).cloned().ok_or("call through a value that is not a function")?;
                if !sigs.contains(&sig) {
                    sigs.push(sig);
                }
            }
        }
    }
    let any = module.funcs.first().ok_or("an empty module")?;
    for (k, (ps, rs)) in sigs.iter().enumerate() {
        let mut params = vec![cx.ptr_ty, cx.i64t, cx.i64t];
        params.extend(ps.iter().map(|&t| cx.lty(any, t)));
        let ret = cx.ret_type(any, rs);
        let fty = cx.m.fn_ty(ret, params);
        let id = cx.m.function(&format!("__dispatch{}", k), fty, false);
        cx.dispatchers.insert((ps.clone(), rs.clone()), (id, fty));
    }
    let has_kernel = module.funcs.iter().position(|f| f.name == "__kernel").is_some_and(|i| !out[i]);
    let kernel = if has_kernel {
        let kf = module.func("__kernel").unwrap();
        cx.kernel_arity = kf.params.len();
        if cx.kernel_arity != 3 && cx.kernel_arity != 5 {
            return Err("__kernel takes (mem, area, id) or (mem, area, id, lane, group)".into());
        }
        let mut params = vec![ptr_ty, ptr_ty, i32t];
        if cx.kernel_arity == 5 {
            params.extend([i32t, i32t]);
        }
        let fty = cx.m.fn_ty(void, params);
        Some(cx.m.function("__kernel", fty, false))
    } else {
        None
    };
    for (i, f) in module.funcs.iter().enumerate() {
        if out[i] {
            continue;
        }
        for inst in f.blocks.iter().flat_map(|b| &b.insts) {
            if let Inst::Call { callee, .. } = inst {
                if let Some(native) = natives.get(callee) {
                    let key = native.rule.lines[0].template.clone().unwrap_or_default();
                    if key == "air.wg.barrier" && !cx.decls.contains_key(&key) {
                        let fty = cx.m.fn_ty(void, vec![i32t, i32t]);
                        let id = cx.m.function(&key, fty, true);
                        cx.m.functions[id].attrs.push(ATTR_CONVERGENT);
                        cx.decls.insert(key.clone(), (id, fty));
                    } else if key.starts_with("air.") && !key.starts_with("air.group") {
                        // the scalar intrinsic, or its vector form for a
                        // call on vectors
                        let lanes = if let Inst::Call { dsts, .. } = inst { dsts.first().and_then(|&d| f.vector(f.ty(d))).map(|(_, n)| n) } else { None };
                        let key = match (lanes, key.rsplit_once('.')) {
                            (Some(n), Some((stem, suffix))) => format!("{}.v{}{}", stem, n, suffix),
                            _ => key.clone(),
                        };
                        if !cx.decls.contains_key(&key) {
                            let vec = |cx: &mut Cx, t: usize| match lanes {
                                Some(n) => cx.m.ty(BType::Vector(t, n as u64)),
                                None => t,
                            };
                            let ret = cx.float_ty(native.ret_bits);
                            let ret = vec(&mut cx, ret);
                            let params: Vec<usize> = native.arg_bits.iter().map(|&b| { let t = cx.float_ty(b); vec(&mut cx, t) }).collect();
                            let fty = cx.m.fn_ty(ret, params);
                            let id = cx.m.function(&key, fty, true);
                            cx.decls.insert(key.clone(), (id, fty));
                        }
                    }
                }
            }
        }
    }

    // bodies
    for (i, f) in module.funcs.iter().enumerate() {
        if out[i] {
            continue;
        }
        cx.emit_function(f, i).map_err(|e| format!("{}: {}", f.name, e))?;
    }
    for (k, (ps, rs)) in sigs.iter().enumerate() {
        cx.emit_dispatcher(k, ps, rs)?;
    }
    if let Some(kid) = kernel {
        cx.emit_kernel(kid, data_size, slab)?;
    }

    let bitcode = cx.m.write();
    let kernels: &[&str] = if has_kernel { &["__kernel"] } else { &[] };
    let metallib = bitcode::metallib(&bitcode, kernels);
    Ok(Compiled { bitcode, metallib, layout: Layout { data, slab }, skipped, has_kernel })
}

struct Cx<'a> {
    m: BModule,
    natives: &'a Natives,
    module: &'a Module,
    ptr_ty: usize,
    void: usize,
    i32t: usize,
    i64t: usize,
    /// SSA function name -> (bitcode function, its type)
    fn_ids: HashMap<String, (usize, usize)>,
    /// intrinsic name -> (function, type)
    decls: HashMap<String, (usize, usize)>,
    taken: Vec<String>,
    data_offsets: HashMap<String, usize>,
    scratch_of: Vec<i64>,
    dispatchers: HashMap<(Vec<Type>, Vec<Type>), (usize, usize)>,
    /// the threadgroup array, if any group operation is used
    group: Option<usize>,
    /// how many arguments the program's kernel takes (3 or 5)
    kernel_arity: usize,
}

/// a function body being emitted
struct Fx<'b> {
    b: FnBuilder,
    f: &'b Function,
    vals: HashMap<u32, usize>,
    env: usize,
    slab: usize,
    /// slab + this function's scratch: what callees get
    next_slab: Option<usize>,
    my_scratch: i64,
    scratch: HashMap<ValueId, i64>,
    /// (from block, target param, value): phi incoming, filled at the end
    branch_args: Vec<(usize, ValueId, ValueId)>,
    cur_block: usize,
}

/// the container of a width: i1, else 8/16/32/64 bits (a multiple of
/// 64 beyond that)
fn cont(bits: u32) -> u32 {
    if bits <= 1 {
        1
    } else if bits <= 8 {
        8
    } else if bits <= 16 {
        16
    } else if bits <= 32 {
        32
    } else {
        64 * bits.div_ceil(64)
    }
}

impl Cx<'_> {
    fn lty(&mut self, f: &Function, t: Type) -> usize {
        match t {
            Type::Int { bits, .. } => self.m.int(cont(bits as u32)),
            Type::Ptr | Type::TPtr(_) | Type::Fn(_) => self.i64t,
            Type::Pack(_) => {
                let w = f.width(t).unwrap_or(64);
                self.m.int(cont(w))
            }
            Type::Struct(_) => match f.vector(t) {
                Some((lane, n)) => {
                    let l = self.lty(f, lane);
                    self.m.ty(BType::Vector(l, n as u64))
                }
                None => unreachable!("structs are lowered before emission"),
            },
            Type::Array(_) | Type::AInt | Type::AUInt => unreachable!("lowered before emission"),
        }
    }

    /// a vector's lane type, else the type itself
    fn scalar_of(&self, f: &Function, t: Type) -> Type {
        f.vector(t).map_or(t, |(l, _)| l)
    }

    /// the constant v of type t: an integer, or a vector of it in every
    /// lane
    fn konst(&mut self, f: &Function, t: Type, v: i64) -> usize {
        match f.vector(t) {
            Some((lane, n)) => {
                let lt = self.lty(f, lane);
                let e = self.m.const_int(lt, v);
                let vt = self.lty(f, t);
                self.m.const_agg(vt, vec![e; n as usize])
            }
            None => {
                let lt = self.lty(f, t);
                self.m.const_int(lt, v)
            }
        }
    }

    fn float_ty(&mut self, bits: u32) -> usize {
        match bits {
            16 => self.m.ty(BType::Half),
            32 => self.m.ty(BType::Float),
            _ => self.m.ty(BType::Double),
        }
    }

    fn ret_type(&mut self, f: &Function, rets: &[Type]) -> usize {
        match rets.len() {
            0 => self.void,
            1 => self.lty(f, rets[0]),
            _ => {
                let elts: Vec<usize> = rets.iter().map(|&t| self.lty(f, t)).collect();
                self.m.ty(BType::Struct(elts))
            }
        }
    }

    /// a value of type t back in canonical form in its container: the
    /// bits above its width zero (unsigned) or its sign (signed)
    fn normalize(&mut self, fx: &mut Fx, v: usize, t: Type) -> usize {
        let f = fx.f;
        let st = self.scalar_of(f, t);
        let Some(bits) = f.width(st) else { return v };
        let c = cont(bits);
        if bits == c || bits > 64 {
            return v;
        }
        if f.repr(st).signed() {
            let sh = self.konst(f, t, (c - bits) as i64);
            let up = fx.b.push(&self.m, B::Bin { op: OP_SHL, lhs: v, rhs: sh, flags: 0 });
            fx.b.push(&self.m, B::Bin { op: OP_ASHR, lhs: up, rhs: sh, flags: 0 })
        } else {
            let mask = self.konst(f, t, ((1i128 << bits) - 1) as i64);
            fx.b.push(&self.m, B::Bin { op: OP_AND, lhs: v, rhs: mask, flags: 0 })
        }
    }

    fn fn_type(&mut self, f: &Function) -> usize {
        let mut params = vec![self.ptr_ty, self.i64t];
        for &p in &f.params {
            let t = f.ty(p);
            params.push(self.lty(f, t));
        }
        let ret = self.ret_type(f, &f.rets);
        self.m.fn_ty(ret, params)
    }

    fn emit_function(&mut self, f: &Function, fi: usize) -> Result<(), String> {
        let (fid, _) = self.fn_ids[&f.name];
        let mut b = FnBuilder::new(&mut self.m, fid);
        // PROBE_AIR_STUB=n: bodies from the n-th on return nothing (an
        // undef), to find which one a compiler rejects
        // PROBE_AIR_KEEP=a,b: only those bodies are real
        let keep = std::env::var("PROBE_AIR_KEEP").ok().map(|v| v.split(',').any(|k| k == f.name));
        // (with both, a kept body is real whatever its index)
        if let Some(n) = std::env::var("PROBE_AIR_STUB").ok().and_then(|v| v.parse::<usize>().ok()).or(if keep == Some(false) { Some(0) } else { None }) {
            if fi == n {
                eprintln!("PROBE_AIR_STUB: bodies from #{} ({}) on are stubs", n, f.name);
            }
            if fi >= n && keep != Some(true) {
                let fty = self.m.functions[fid].ty;
                let ret = match &self.m.types[fty] {
                    BType::Fn(r, _) => *r,
                    _ => unreachable!(),
                };
                let val = if matches!(self.m.types[ret], BType::Void) { None } else { Some(self.m.const_undef(ret)) };
                b.push(&self.m, B::Ret { val });
                b.finish(&mut self.m);
                return Ok(());
            }
        }
        let env = b.arg(&self.m, 0);
        let slab = b.arg(&self.m, 1);
        let mut vals = HashMap::new();
        for (i, &p) in f.params.iter().enumerate() {
            vals.insert(p.0, b.arg(&self.m, 2 + i));
        }
        for bi in 1..f.blocks.len() {
            let nb = b.block();
            debug_assert_eq!(nb, bi);
        }
        // a phi per block parameter, incoming filled at the end
        for (bi, block) in f.blocks.iter().enumerate() {
            for &p in &block.params {
                let ty = self.lty(f, f.ty(p));
                let id = b.push_in(&self.m, bi, B::Phi { ty, incoming: Vec::new() });
                vals.insert(p.0, id);
            }
        }
        let (scratch, _) = crate::emit::scratch_layout(f, 0);
        let mut fx = Fx { b, f, vals, env, slab, next_slab: None, my_scratch: self.scratch_of[fi], scratch, branch_args: Vec::new(), cur_block: 0 };
        // PROBE_AIR_CUT=k: in a kept function, blocks from the k-th on
        // are `unreachable` (to find which block a compiler rejects)
        let cut = if keep == Some(true) { std::env::var("PROBE_AIR_CUT").ok().and_then(|v| v.parse::<usize>().ok()) } else { None };
        for (bi, block) in f.blocks.iter().enumerate() {
            fx.b.at(bi);
            fx.cur_block = bi;
            if cut.is_some_and(|k| bi >= k) {
                fx.b.push(&self.m, B::Unreachable);
                continue;
            }
            for inst in &block.insts {
                self.emit_inst(&mut fx, inst)?;
            }
        }
        // the phis: the first instructions of their block, in parameter order
        let mut incoming: HashMap<u32, Vec<(usize, usize)>> = HashMap::new();
        let branch_args = std::mem::take(&mut fx.branch_args);
        for (from, param, val) in branch_args {
            let v = self.value(&mut fx, val)?;
            incoming.entry(param.0).or_default().push((v, from));
        }
        for (bi, block) in f.blocks.iter().enumerate() {
            for (k, &p) in block.params.iter().enumerate() {
                if let B::Phi { incoming: inc, .. } = &mut fx.b.blocks[bi].insts[k] {
                    *inc = incoming.remove(&p.0).unwrap_or_default();
                    // a block reached twice from one predecessor with
                    // different arguments has no phi
                    let mut seen: Vec<usize> = Vec::new();
                    for (v, from) in inc.iter() {
                        if seen.contains(from) {
                            if inc.iter().any(|(v2, f2)| f2 == from && v2 != v) {
                                return Err(format!("block {} is branched to twice from one block with different arguments; AIR cannot express that", block.name));
                            }
                        }
                        seen.push(*from);
                    }
                    inc.dedup();
                }
            }
        }
        fx.b.finish(&mut self.m);
        Ok(())
    }

    /// the bitcode value of an SSA value
    fn value(&mut self, fx: &mut Fx, v: ValueId) -> Result<usize, String> {
        fx.vals.get(&v.0).copied().ok_or_else(|| format!("value {} used before it is defined", fx.f.value(v).name))
    }

    fn const_i64(&mut self, v: i64) -> usize {
        let t = self.i64t;
        self.m.const_int(t, v)
    }

    /// slab + this function's own scratch, computed once
    fn next_slab(&mut self, fx: &mut Fx) -> usize {
        if let Some(s) = fx.next_slab {
            return s;
        }
        let off = self.const_i64(fx.my_scratch);
        let s = fx.b.push(&self.m, B::Bin { op: OP_ADD, lhs: fx.slab, rhs: off, flags: 0 });
        fx.next_slab = Some(s);
        s
    }

    /// the offset base + off + index * step
    fn address(&mut self, fx: &mut Fx, addr: ValueId, off: i64, index: Option<(ValueId, u32)>) -> Result<usize, String> {
        let mut p = self.value(fx, addr)?;
        let mut total: Option<usize> = None;
        if let Some((i, step)) = index {
            let iv = self.value(fx, i)?;
            let stepc = self.const_i64(step as i64);
            let scaled = fx.b.push(&self.m, B::Bin { op: OP_MUL, lhs: iv, rhs: stepc, flags: 0 });
            total = Some(scaled);
        }
        if off != 0 {
            let c = self.const_i64(off);
            total = Some(match total {
                Some(t) => fx.b.push(&self.m, B::Bin { op: OP_ADD, lhs: t, rhs: c, flags: 0 }),
                None => c,
            });
        }
        if let Some(t) = total {
            p = fx.b.push(&self.m, B::Bin { op: OP_ADD, lhs: p, rhs: t, flags: 0 });
        }
        Ok(p)
    }

    /// the device pointer at an offset, typed for a value: iN, or i64
    /// for a pointer/function value
    fn mem_ptr(&mut self, fx: &mut Fx, p: usize, t: Type) -> Result<(usize, usize), String> {
        let mt = match t {
            Type::Ptr | Type::TPtr(_) | Type::Fn(_) => self.i64t,
            _ if fx.f.vector(t).is_some() => {
                // lanes as their containers, a byte each for u1 (the
                // struct's layout); the callers convert
                let (lane, n) = fx.f.vector(t).unwrap();
                let w = fx.f.width(lane).ok_or_else(|| format!("no memory width for {}", fx.f.tyname(lane)))?;
                let lt = self.m.int(cont(w).max(8));
                self.m.ty(BType::Vector(lt, n as u64))
            }
            _ => {
                let w = fx.f.width(t).ok_or_else(|| format!("no memory width for {}", fx.f.tyname(t)))?;
                self.m.int(cont(w))
            }
        };
        let pt = self.m.ptr(mt, 1);
        let i8t = self.m.int(8);
        let env = fx.env;
        let bp = fx.b.push(&self.m, B::Gep { elem_ty: i8t, base: env, idx: vec![p], inbounds: true });
        let cast = fx.b.push(&self.m, B::Cast { op: CAST_BITCAST, val: bp, ty: pt });
        Ok((cast, mt))
    }

    fn emit_inst(&mut self, fx: &mut Fx, inst: &Inst) -> Result<(), String> {
        let f = fx.f;
        match inst {
            Inst::IConst { dst, imm } => {
                // a literal of a vector type is that value in every lane
                let t = f.ty(*dst);
                let st = self.scalar_of(f, t);
                let v = crate::opt::norm(f.repr(st), *imm as i64);
                let c = self.konst(f, t, v);
                fx.vals.insert(dst.0, c);
            }
            Inst::Bin { op, dst, lhs, rhs } => {
                let t = f.ty(*dst);
                let st = self.scalar_of(f, t);
                let signed = f.repr(st).signed();
                let code = match op {
                    BinOp::IAdd => OP_ADD,
                    BinOp::ISub => OP_SUB,
                    BinOp::IMul => OP_MUL,
                    BinOp::Div => if signed { OP_SDIV } else { OP_UDIV },
                    BinOp::Rem => if signed { OP_SREM } else { OP_UREM },
                    BinOp::And => OP_AND,
                    BinOp::Or => OP_OR,
                    BinOp::Xor => OP_XOR,
                    BinOp::Shl => OP_SHL,
                    BinOp::Shr => if signed { OP_ASHR } else { OP_LSHR },
                };
                let (a, mut c) = (self.value(fx, *lhs)?, self.value(fx, *rhs)?);
                let bits = f.width(st).unwrap_or(64);
                // LLVM has no answer for MIN / -1 (the IR: MIN) or MIN % -1
                // (0): divide by rhs + 2m instead, m = (rhs == -1), and
                // negate the quotient back, (q ^ -m) + m — in a full
                // container only; a narrower type's MIN / -1 fits
                let m = if signed && matches!(op, BinOp::Div | BinOp::Rem) && bits == cont(bits) {
                    let lt = self.lty(f, t);
                    let minus1 = self.konst(f, t, -1);
                    let two = self.konst(f, t, 2);
                    let is = fx.b.push(&self.m, B::Cmp { pred: 32, lhs: c, rhs: minus1 });
                    let m = fx.b.push(&self.m, B::Cast { op: CAST_ZEXT, val: is, ty: lt });
                    let m2 = fx.b.push(&self.m, B::Bin { op: OP_MUL, lhs: m, rhs: two, flags: 0 });
                    c = fx.b.push(&self.m, B::Bin { op: OP_ADD, lhs: c, rhs: m2, flags: 0 });
                    Some(m)
                } else {
                    None
                };
                let mut v = fx.b.push(&self.m, B::Bin { op: code, lhs: a, rhs: c, flags: 0 });
                if let (Some(m), BinOp::Div) = (m, op) {
                    let zero = self.konst(f, t, 0);
                    let neg = fx.b.push(&self.m, B::Bin { op: OP_SUB, lhs: zero, rhs: m, flags: 0 });
                    let x = fx.b.push(&self.m, B::Bin { op: OP_XOR, lhs: v, rhs: neg, flags: 0 });
                    v = fx.b.push(&self.m, B::Bin { op: OP_ADD, lhs: x, rhs: m, flags: 0 });
                }
                // what can carry out of the width comes back canonical
                if matches!(op, BinOp::IAdd | BinOp::ISub | BinOp::IMul | BinOp::Div | BinOp::Rem | BinOp::Shl) {
                    v = self.normalize(fx, v, t);
                }
                fx.vals.insert(dst.0, v);
            }
            Inst::ICmp { cond, dst, lhs, rhs } => {
                let t = self.scalar_of(f, f.ty(*lhs));
                let signed = t.is_int() && f.repr(t).signed();
                let pred = match (cond, signed) {
                    (Cond::Eq, _) => 32,
                    (Cond::Ne, _) => 33,
                    (Cond::Gt, false) => 34,
                    (Cond::Ge, false) => 35,
                    (Cond::Lt, false) => 36,
                    (Cond::Le, false) => 37,
                    (Cond::Gt, true) => 38,
                    (Cond::Ge, true) => 39,
                    (Cond::Lt, true) => 40,
                    (Cond::Le, true) => 41,
                };
                let (a, c) = (self.value(fx, *lhs)?, self.value(fx, *rhs)?);
                let v = fx.b.push(&self.m, B::Cmp { pred, lhs: a, rhs: c });
                fx.vals.insert(dst.0, v);
            }
            Inst::Cast { op, dst, src } => {
                let (td, ts) = (f.ty(*dst), f.ty(*src));
                let s = self.value(fx, *src)?;
                // the container resized (extended by the source's sign),
                // then the value made canonical for its new width and
                // signedness — for `cast` too: the same bits in a signed
                // type are a different container value
                let (sd, ss) = (self.scalar_of(f, td), self.scalar_of(f, ts));
                let (wd, ws) = (f.width(sd).unwrap_or(64), f.width(ss).unwrap_or(64));
                let (cd, cs) = (cont(wd), cont(ws));
                let lt = self.lty(f, td);
                let mut v = if cd < cs {
                    fx.b.push(&self.m, B::Cast { op: CAST_TRUNC, val: s, ty: lt })
                } else if cd > cs {
                    let ext = if *op == CastOp::Conv && f.repr(ss).signed() { CAST_SEXT } else { CAST_ZEXT };
                    fx.b.push(&self.m, B::Cast { op: ext, val: s, ty: lt })
                } else {
                    s
                };
                if wd < cd {
                    v = self.normalize(fx, v, td);
                }
                fx.vals.insert(dst.0, v);
            }
            Inst::Pack { dst: v, .. } | Inst::Set { dst: v, .. } if f.vector(f.ty(*v)).is_some() => {
                self.emit_vector_op(fx, inst)?;
            }
            Inst::Unpack { src: v, .. } | Inst::Get { src: v, .. } if f.vector(f.ty(*v)).is_some() => {
                self.emit_vector_op(fx, inst)?;
            }
            Inst::Pack { .. } | Inst::Unpack { .. } | Inst::Get { .. } | Inst::Set { .. } => {
                // packs: bit fields in an integer
                self.emit_pack_op(fx, inst)?;
            }
            Inst::Load { dst, addr, off, index } => {
                let p = self.address(fx, *addr, *off, *index)?;
                let t = f.ty(*dst);
                let (pt, mt) = self.mem_ptr(fx, p, t)?;
                let mut v = fx.b.push(&self.m, B::Load { ptr: pt, ty: mt, align: 1 });
                if f.vector(t).is_some_and(|(l, _)| l == Type::U1) {
                    let lt = self.lty(f, t);
                    v = fx.b.push(&self.m, B::Cast { op: CAST_TRUNC, val: v, ty: lt });
                }
                // memory holds the container; what was put there by
                // something else may not be canonical
                let v = self.normalize(fx, v, t);
                fx.vals.insert(dst.0, v);
            }
            Inst::Store { val, addr, off, index } => {
                let p = self.address(fx, *addr, *off, *index)?;
                let t = f.ty(*val);
                let mut v = self.value(fx, *val)?;
                let (pt, mt) = self.mem_ptr(fx, p, t)?;
                if f.vector(t).is_some_and(|(l, _)| l == Type::U1) {
                    v = fx.b.push(&self.m, B::Cast { op: CAST_ZEXT, val: v, ty: mt });
                }
                fx.b.push(&self.m, B::Store { ptr: pt, val: v, align: 1 });
            }
            Inst::PtrAdd { dst, base, off } => {
                let (p, o) = (self.value(fx, *base)?, self.value(fx, *off)?);
                let v = fx.b.push(&self.m, B::Bin { op: OP_ADD, lhs: p, rhs: o, flags: 0 });
                fx.vals.insert(dst.0, v);
            }
            Inst::Addr { dst, name } => {
                let off = *self.data_offsets.get(name.as_str()).ok_or_else(|| format!("no data named {}", name))? as i64;
                let c = self.const_i64(off);
                fx.vals.insert(dst.0, c);
            }
            Inst::Scratch { dst, .. } => {
                let off = fx.scratch[dst];
                let c = self.const_i64(off);
                let slab = fx.slab;
                let v = fx.b.push(&self.m, B::Bin { op: OP_ADD, lhs: slab, rhs: c, flags: 0 });
                fx.vals.insert(dst.0, v);
            }
            Inst::Platform { dst, name } => {
                let v = *self.natives.consts.get(name).ok_or_else(|| format!("the platform has no constant '{}'", name))?;
                let t = f.ty(*dst);
                let lt = self.lty(f, t);
                let c = self.m.const_int(lt, v);
                fx.vals.insert(dst.0, c);
            }
            Inst::FnAddr { dst, name } => {
                let idx = self.taken.iter().position(|t| t == name).ok_or("function not in the table")? as i64;
                let c = self.const_i64(idx);
                fx.vals.insert(dst.0, c);
            }
            // a platform rule names the operation, operands or not (as on
            // wasm): the emitter knows it by its key
            Inst::Call { dsts, callee, args } if self.natives.get(callee).is_some() => {
                self.emit_rule(fx, dsts, callee, args)?;
            }
            Inst::Call { dsts, callee, args } if args.iter().any(|&x| f.vector(f.ty(x)).is_some()) => {
                // a lane operation the library does: once per lane, the
                // lanes taken out and the results put back
                let (fid, fty) = *self.fn_ids.get(callee).ok_or_else(|| format!("call to {}, which is not in this module (left out?)", callee))?;
                let fv = self.m.function_value(fid);
                let ns = self.next_slab(fx);
                let n = args.iter().find_map(|&x| f.vector(f.ty(x))).map(|(_, n)| n).unwrap();
                let i32t = self.i32t;
                let argv: Vec<usize> = args.iter().map(|&x| self.value(fx, x)).collect::<Result<_, _>>()?;
                let mut results: Vec<usize> = dsts
                    .iter()
                    .map(|&d| {
                        let lt = self.lty(f, f.ty(d));
                        self.m.const_undef(lt)
                    })
                    .collect();
                for k in 0..n {
                    let idx = self.m.const_int(i32t, k as i64);
                    let mut a = vec![fx.env, ns];
                    for (&x, &v) in args.iter().zip(&argv) {
                        a.push(if f.vector(f.ty(x)).is_some() { fx.b.push(&self.m, B::ExtractElt { vec: v, idx }) } else { v });
                    }
                    let r = fx.b.push(&self.m, B::Call { fn_ty: fty, callee: fv, args: a });
                    for (i, res) in results.iter_mut().enumerate() {
                        let lane = if dsts.len() == 1 { r } else { fx.b.push(&self.m, B::ExtractVal { agg: r, idx: i as u32 }) };
                        *res = fx.b.push(&self.m, B::InsertElt { vec: *res, elt: lane, idx });
                    }
                }
                for (&d, &r) in dsts.iter().zip(&results) {
                    fx.vals.insert(d.0, r);
                }
            }
            Inst::Call { dsts, callee, args } => {
                let (fid, fty) = *self.fn_ids.get(callee).ok_or_else(|| format!("call to {}, which is not in this module (left out?)", callee))?;
                let fv = self.m.function_value(fid);
                let ns = self.next_slab(fx);
                let mut a = vec![fx.env, ns];
                for &x in args {
                    a.push(self.value(fx, x)?);
                }
                let r = fx.b.push(&self.m, B::Call { fn_ty: fty, callee: fv, args: a });
                self.bind_results(fx, dsts, r);
            }
            Inst::CallInd { dsts, callee, args } => {
                let sig = f.sig(f.ty(*callee)).cloned().ok_or("call through a non-function")?;
                let (did, dty) = *self.dispatchers.get(&sig).ok_or("no dispatcher for this signature")?;
                let dv = self.m.function_value(did);
                let ns = self.next_slab(fx);
                let idx = self.value(fx, *callee)?;
                let mut a = vec![fx.env, ns, idx];
                for &x in args {
                    a.push(self.value(fx, x)?);
                }
                let r = fx.b.push(&self.m, B::Call { fn_ty: dty, callee: dv, args: a });
                self.bind_results(fx, dsts, r);
            }
            Inst::Check { cond } => {
                // holds: on; does not: the program is over (unreachable)
                let c = self.value(fx, *cond)?;
                let ok = fx.b.block();
                let bad = fx.b.block();
                fx.b.push(&self.m, B::CondBr { cond: c, then: ok, els: bad });
                fx.b.at(bad);
                fx.b.push(&self.m, B::Unreachable);
                fx.b.at(ok);
                // the rest of the block is `ok`: branches from here on
                // come from it
                fx.cur_block = ok;
            }
            Inst::Jmp { target, args } => {
                let tb = &f.blocks[target.0 as usize];
                for (&p, &a) in tb.params.iter().zip(args) {
                    fx.branch_args.push((fx.cur_block, p, a));
                }
                let t = target.0 as usize;
                fx.b.push(&self.m, B::Br { target: t });
            }
            Inst::Br { cond, then_target, then_args, else_target, else_args } => {
                let c = self.value(fx, *cond)?;
                let (t, e) = (then_target.0 as usize, else_target.0 as usize);
                if t == e {
                    // both arms to one block, each with its own arguments
                    // (a select): a phi wants one edge per predecessor, so
                    // each arm gets a block of its own to be that
                    let (bt, be) = (fx.b.block(), fx.b.block());
                    fx.b.push(&self.m, B::CondBr { cond: c, then: bt, els: be });
                    for (blk, args) in [(bt, then_args), (be, else_args)] {
                        fx.b.at(blk);
                        for (&p, &a) in f.blocks[t].params.iter().zip(args) {
                            fx.branch_args.push((blk, p, a));
                        }
                        fx.b.push(&self.m, B::Br { target: t });
                    }
                    fx.b.at(fx.cur_block);
                    return Ok(());
                }
                for (&p, &a) in f.blocks[t].params.iter().zip(then_args) {
                    fx.branch_args.push((fx.cur_block, p, a));
                }
                for (&p, &a) in f.blocks[e].params.iter().zip(else_args) {
                    fx.branch_args.push((fx.cur_block, p, a));
                }
                fx.b.push(&self.m, B::CondBr { cond: c, then: t, els: e });
            }
            Inst::Ret { vals } => match vals.len() {
                0 => {
                    fx.b.push(&self.m, B::Ret { val: None });
                }
                1 => {
                    let v = self.value(fx, vals[0])?;
                    fx.b.push(&self.m, B::Ret { val: Some(v) });
                }
                _ => {
                    let st = self.ret_type(f, &f.rets);
                    let mut agg = self.m.const_undef(st);
                    for (i, &v) in vals.iter().enumerate() {
                        let x = self.value(fx, v)?;
                        agg = fx.b.push(&self.m, B::InsertVal { agg, val: x, idx: i as u32 });
                    }
                    fx.b.push(&self.m, B::Ret { val: Some(agg) });
                }
            },
        }
        Ok(())
    }

    /// the results of a call: one value, or the fields of a struct
    fn bind_results(&mut self, fx: &mut Fx, dsts: &[ValueId], r: usize) {
        match dsts.len() {
            0 => {}
            1 => {
                fx.vals.insert(dsts[0].0, r);
            }
            _ => {
                for (i, &d) in dsts.iter().enumerate() {
                    let v = fx.b.push(&self.m, B::ExtractVal { agg: r, idx: i as u32 });
                    fx.vals.insert(d.0, v);
                }
            }
        }
    }

    /// vectors as LLVM's: lanes go in and out by index
    fn emit_vector_op(&mut self, fx: &mut Fx, inst: &Inst) -> Result<(), String> {
        let f = fx.f;
        let i32t = self.i32t;
        match inst {
            Inst::Pack { dst, args } => {
                let lt = self.lty(f, f.ty(*dst));
                let mut acc = self.m.const_undef(lt);
                for (k, &a) in args.iter().enumerate() {
                    let v = self.value(fx, a)?;
                    let idx = self.m.const_int(i32t, k as i64);
                    acc = fx.b.push(&self.m, B::InsertElt { vec: acc, elt: v, idx });
                }
                fx.vals.insert(dst.0, acc);
            }
            Inst::Unpack { dsts, src } => {
                let v = self.value(fx, *src)?;
                for (k, &d) in dsts.iter().enumerate() {
                    let idx = self.m.const_int(i32t, k as i64);
                    let lane = fx.b.push(&self.m, B::ExtractElt { vec: v, idx });
                    fx.vals.insert(d.0, lane);
                }
            }
            Inst::Get { dst, src, field } => {
                let v = self.value(fx, *src)?;
                let idx = self.m.const_int(i32t, *field as i64);
                let lane = fx.b.push(&self.m, B::ExtractElt { vec: v, idx });
                fx.vals.insert(dst.0, lane);
            }
            Inst::Set { dst, src, field, val } => {
                let (v, x) = (self.value(fx, *src)?, self.value(fx, *val)?);
                let idx = self.m.const_int(i32t, *field as i64);
                let r = fx.b.push(&self.m, B::InsertElt { vec: v, elt: x, idx });
                fx.vals.insert(dst.0, r);
            }
            _ => unreachable!(),
        }
        Ok(())
    }

    /// packs as integers: a field is a shift and a mask
    fn emit_pack_op(&mut self, fx: &mut Fx, inst: &Inst) -> Result<(), String> {
        let f = fx.f;
        match inst {
            Inst::Pack { dst, args } => {
                let t = f.ty(*dst);
                let lt = self.lty(f, t);
                let p = f.pack(t).ok_or("pack of a non-pack")?.clone();
                let mut acc = self.m.const_int(lt, 0);
                for (k, &a) in args.iter().enumerate() {
                    let (off, fty) = (p.offsets[k], p.fields[k].1);
                    let fw = f.width(fty).unwrap_or(64);
                    let mut v = self.value(fx, a)?;
                    if cont(fw) < cont(p.width) {
                        v = fx.b.push(&self.m, B::Cast { op: CAST_ZEXT, val: v, ty: lt });
                    }
                    if fw < cont(p.width) {
                        // the field's bits only (a signed field may carry sign bits)
                        let mask = self.m.const_int(lt, ((1i128 << fw) - 1) as i64);
                        v = fx.b.push(&self.m, B::Bin { op: OP_AND, lhs: v, rhs: mask, flags: 0 });
                    }
                    if off > 0 {
                        let sh = self.m.const_int(lt, off as i64);
                        v = fx.b.push(&self.m, B::Bin { op: OP_SHL, lhs: v, rhs: sh, flags: 0 });
                    }
                    acc = fx.b.push(&self.m, B::Bin { op: OP_OR, lhs: acc, rhs: v, flags: 0 });
                }
                fx.vals.insert(dst.0, acc);
            }
            Inst::Unpack { dsts, src } => {
                for (k, &d) in dsts.iter().enumerate() {
                    let v = self.field_of(fx, *src, k as u32)?;
                    fx.vals.insert(d.0, v);
                }
            }
            Inst::Get { dst, src, field } => {
                let v = self.field_of(fx, *src, *field)?;
                fx.vals.insert(dst.0, v);
            }
            Inst::Set { dst, src, field, val } => {
                let t = f.ty(*src);
                let lt = self.lty(f, t);
                let p = f.pack(t).ok_or("set on a non-pack")?.clone();
                let (off, fty) = (p.offsets[*field as usize], p.fields[*field as usize].1);
                let fw = f.width(fty).unwrap_or(64);
                let s = self.value(fx, *src)?;
                let field_mask = ((1i128 << fw) - 1) as i64;
                let hole = self.m.const_int(lt, !(field_mask << off) & (if p.width == 64 { -1 } else { ((1i128 << p.width) - 1) as i64 }));
                let cleared = fx.b.push(&self.m, B::Bin { op: OP_AND, lhs: s, rhs: hole, flags: 0 });
                let mut v = self.value(fx, *val)?;
                if cont(fw) < cont(p.width) {
                    v = fx.b.push(&self.m, B::Cast { op: CAST_ZEXT, val: v, ty: lt });
                }
                if fw < cont(p.width) {
                    let mask = self.m.const_int(lt, field_mask);
                    v = fx.b.push(&self.m, B::Bin { op: OP_AND, lhs: v, rhs: mask, flags: 0 });
                }
                if off > 0 {
                    let sh = self.m.const_int(lt, off as i64);
                    v = fx.b.push(&self.m, B::Bin { op: OP_SHL, lhs: v, rhs: sh, flags: 0 });
                }
                let r = fx.b.push(&self.m, B::Bin { op: OP_OR, lhs: cleared, rhs: v, flags: 0 });
                fx.vals.insert(dst.0, r);
            }
            _ => unreachable!(),
        }
        Ok(())
    }

    /// field k of a pack value: shifted down, then narrowed (sign-extended
    /// for a signed field, as the IR's canonical form wants)
    fn field_of(&mut self, fx: &mut Fx, src: ValueId, k: u32) -> Result<usize, String> {
        let f = fx.f;
        let t = f.ty(src);
        let lt = self.lty(f, t);
        let p = f.pack(t).ok_or("get on a non-pack")?.clone();
        let (off, fty) = (p.offsets[k as usize], p.fields[k as usize].1);
        let fw = f.width(fty).unwrap_or(64);
        let mut v = self.value(fx, src)?;
        if off > 0 {
            let sh = self.m.const_int(lt, off as i64);
            v = fx.b.push(&self.m, B::Bin { op: OP_LSHR, lhs: v, rhs: sh, flags: 0 });
        }
        if cont(fw) < cont(p.width) {
            let ft = self.lty(f, fty);
            v = fx.b.push(&self.m, B::Cast { op: CAST_TRUNC, val: v, ty: ft });
        }
        // the bits above the field are the next field's: canonical again
        let v = self.normalize(fx, v, fty);
        Ok(v)
    }

    /// a platform rule: the arguments as floats, the operation, the
    /// result as bits
    fn emit_rule(&mut self, fx: &mut Fx, dsts: &[ValueId], callee: &str, args: &[ValueId]) -> Result<(), String> {
        let native = self.natives.get(callee).unwrap();
        let key = native.rule.lines[0].template.clone().unwrap_or_default();
        // the group operations: an offset into the threadgroup array
        if key == "air.wg.barrier" {
            let (id, fty) = self.decls[&key];
            let fv = self.m.function_value(id);
            let (flags, scope) = (self.m.const_int(self.i32t, 2), self.m.const_int(self.i32t, 1));
            fx.b.push(&self.m, B::Call { fn_ty: fty, callee: fv, args: vec![flags, scope] });
            return Ok(());
        }
        if let Some(what) = key.strip_prefix("air.group.") {
            let g = self.group.ok_or("group memory used but not set up")?;
            let gv = self.m.global_value(g);
            let i8t = self.m.int(8);
            let arr = self.m.ty(BType::Array(i8t, GROUP_BYTES));
            let i64t = self.i64t;
            let p64 = self.m.ptr(i64t, 3);
            let zero = self.const_i64(0);
            match what {
                "load" => {
                    let off = self.value(fx, args[0])?;
                    let p = fx.b.push(&self.m, B::Gep { elem_ty: arr, base: gv, idx: vec![zero, off], inbounds: true });
                    let tp = fx.b.push(&self.m, B::Cast { op: CAST_BITCAST, val: p, ty: p64 });
                    let v = fx.b.push(&self.m, B::Load { ptr: tp, ty: i64t, align: 8 });
                    fx.vals.insert(dsts[0].0, v);
                }
                "store" => {
                    let (v, off) = (self.value(fx, args[0])?, self.value(fx, args[1])?);
                    let p = fx.b.push(&self.m, B::Gep { elem_ty: arr, base: gv, idx: vec![zero, off], inbounds: true });
                    let tp = fx.b.push(&self.m, B::Cast { op: CAST_BITCAST, val: p, ty: p64 });
                    fx.b.push(&self.m, B::Store { ptr: tp, val: v, align: 8 });
                }
                _ => return Err(format!("unknown group operation '{}'", key)),
            }
            return Ok(());
        }
        let Some(&dst) = dsts.first() else { return Ok(()) };
        // on vectors, the rule applies to the whole vector: the float
        // types are vectors, an intrinsic its vector form
        let f = fx.f;
        let lanes = f.vector(f.ty(dst)).map(|(_, n)| n);
        let key = match (lanes, key.rsplit_once('.')) {
            (Some(n), Some((stem, suffix))) if key.starts_with("air.") && !key.starts_with("air.wg") => format!("{}.v{}{}", stem, n, suffix),
            _ => key,
        };
        let mut fargs = Vec::new();
        for (j, &a) in args.iter().enumerate() {
            let v = self.value(fx, a)?;
            let bits = native.arg_bits[j];
            let v = if native.arg_class[j].is_some() || bits >= 16 && key != "" {
                let ft = self.float_ty(bits);
                let ft = match lanes {
                    Some(n) => self.m.ty(BType::Vector(ft, n as u64)),
                    None => ft,
                };
                fx.b.push(&self.m, B::Cast { op: CAST_BITCAST, val: v, ty: ft })
            } else {
                v
            };
            fargs.push(v);
        }
        let ret_float = native.ret_bits >= 16 && !key.starts_with("fcmp");
        let r = if let Some(pred) = key.strip_prefix("fcmp.") {
            let code = match pred {
                "oeq" => 1, "ogt" => 2, "oge" => 3, "olt" => 4, "ole" => 5, "one" => 6, "ord" => 7, "uno" => 8,
                "ueq" => 9, "ugt" => 10, "uge" => 11, "ult" => 12, "ule" => 13, "une" => 14,
                _ => return Err(format!("unknown float comparison '{}'", pred)),
            };
            fx.b.push(&self.m, B::Cmp { pred: code, lhs: fargs[0], rhs: fargs[1] })
        } else if let Some((id, fty)) = self.decls.get(&key).copied() {
            let fv = self.m.function_value(id);
            fx.b.push(&self.m, B::Call { fn_ty: fty, callee: fv, args: fargs })
        } else {
            match key.as_str() {
                "fadd" => fx.b.push(&self.m, B::Bin { op: OP_ADD, lhs: fargs[0], rhs: fargs[1], flags: 0 }),
                "fsub" => fx.b.push(&self.m, B::Bin { op: OP_SUB, lhs: fargs[0], rhs: fargs[1], flags: 0 }),
                "fmul" => fx.b.push(&self.m, B::Bin { op: OP_MUL, lhs: fargs[0], rhs: fargs[1], flags: 0 }),
                "fdiv" => fx.b.push(&self.m, B::Bin { op: OP_SDIV, lhs: fargs[0], rhs: fargs[1], flags: 0 }),
                "fneg" => fx.b.push(&self.m, B::Unop { op: 0, val: fargs[0] }),
                other => return Err(format!("rule '{}': no such operation on air", other)),
            }
        };
        let r = if ret_float {
            let it = self.m.int(native.ret_bits);
            let it = match lanes {
                Some(n) => self.m.ty(BType::Vector(it, n as u64)),
                None => it,
            };
            fx.b.push(&self.m, B::Cast { op: CAST_BITCAST, val: r, ty: it })
        } else {
            r
        };
        fx.vals.insert(dst.0, r);
        Ok(())
    }

    /// a dispatcher: switch on the index, call the function, return
    fn emit_dispatcher(&mut self, k: usize, ps: &[Type], rs: &[Type]) -> Result<(), String> {
        let (did, _) = self.dispatchers[&(ps.to_vec(), rs.to_vec())];
        let any = self.module.funcs.first().unwrap();
        let mut b = FnBuilder::new(&mut self.m, did);
        let env = b.arg(&self.m, 0);
        let slab = b.arg(&self.m, 1);
        let idx = b.arg(&self.m, 2);
        let args: Vec<usize> = (0..ps.len()).map(|i| b.arg(&self.m, 3 + i)).collect();
        let targets: Vec<(i64, String)> = self
            .taken
            .iter()
            .enumerate()
            .filter(|(_, t)| {
                let tf = self.module.func(t).unwrap();
                tf.params.iter().map(|&p| tf.ty(p)).collect::<Vec<_>>() == ps && tf.rets == rs && self.fn_ids.contains_key(t.as_str())
            })
            .map(|(i, t)| (i as i64, t.clone()))
            .collect();
        let default = b.block();
        let mut cases = Vec::new();
        for (i, name) in &targets {
            let blk = b.block();
            let c = self.const_i64(*i);
            cases.push((c, blk, name.clone()));
        }
        let i64t = self.i64t;
        b.at(0);
        b.push(&self.m, B::Switch { ty: i64t, val: idx, default, cases: cases.iter().map(|(c, blk, _)| (*c, *blk)).collect() });
        b.at(default);
        b.push(&self.m, B::Unreachable);
        let ret_void = rs.is_empty();
        let _ = any;
        for (_, blk, name) in &cases {
            b.at(*blk);
            let (fid, fty) = self.fn_ids[name];
            let fv = self.m.function_value(fid);
            let mut a = vec![env, slab];
            a.extend(args.iter().copied());
            let r = b.push(&self.m, B::Call { fn_ty: fty, callee: fv, args: a });
            b.push(&self.m, B::Ret { val: if ret_void { None } else { Some(r) } });
        }
        let _ = k;
        b.finish(&mut self.m);
        Ok(())
    }

    /// the kernel: buffers 0 (the program's memory) and 1 (the driver's
    /// parameters: the offset of its area) and the thread's position;
    /// the thread's slab is after the data, by its position
    fn emit_kernel(&mut self, kid: usize, data_size: u64, slab: u64) -> Result<(), String> {
        let (bid, bty) = self.fn_ids["__kernel"];
        let mut b = FnBuilder::new(&mut self.m, kid);
        let mem = b.arg(&self.m, 0);
        let params = b.arg(&self.m, 1);
        let tid = b.arg(&self.m, 2);
        let i64t = self.i64t;
        let id64 = b.push(&self.m, B::Cast { op: CAST_ZEXT, val: tid, ty: i64t });
        let slabc = self.const_i64(slab as i64);
        let off0 = b.push(&self.m, B::Bin { op: OP_MUL, lhs: id64, rhs: slabc, flags: 0 });
        let dsc = self.const_i64(data_size as i64);
        let my_slab = b.push(&self.m, B::Bin { op: OP_ADD, lhs: off0, rhs: dsc, flags: 0 });
        let i64p = self.m.ptr(i64t, 1);
        let pp = b.push(&self.m, B::Cast { op: CAST_BITCAST, val: params, ty: i64p });
        let area = b.push(&self.m, B::Load { ptr: pp, ty: i64t, align: 8 });
        let zero = self.const_i64(0);
        let fv = self.m.function_value(bid);
        let mut args = vec![mem, my_slab, zero, area, id64];
        if self.kernel_arity == 5 {
            let lane = b.arg(&self.m, 3);
            let group = b.arg(&self.m, 4);
            let lane64 = b.push(&self.m, B::Cast { op: CAST_ZEXT, val: lane, ty: i64t });
            let group64 = b.push(&self.m, B::Cast { op: CAST_ZEXT, val: group, ty: i64t });
            args.extend([lane64, group64]);
        }
        b.push(&self.m, B::Call { fn_ty: bty, callee: fv, args });
        b.push(&self.m, B::Ret { val: None });
        b.finish(&mut self.m);

        // the metadata: air.kernel names the arguments
        let i32t = self.i32t;
        let fty = self.m.types[self.m.functions[kid].ty].clone();
        let fty_id = self.m.ty(fty);
        let pfty = self.m.ptr(fty_id, 0);
        let fv = self.m.function_value(kid);
        let fn_md = self.m.md_value(pfty, fv);
        let empty = self.m.md_node(vec![]);
        let c = |m: &mut BModule, v: i64| {
            let ci = m.const_int(i32t, v);
            m.md_value(i32t, ci)
        };
        let (v0, v1, v2, v8) = (c(&mut self.m, 0), c(&mut self.m, 1), c(&mut self.m, 2), c(&mut self.m, 8));
        let s = |m: &mut BModule, t: &str| Some(m.md_str(t));
        let buffer_arg = |m: &mut BModule, index: usize, name: &str| {
            let vi = c(m, index as i64);
            let items = vec![
                Some(vi), s(m, "air.buffer"), s(m, "air.location_index"), Some(vi), Some(v1), s(m, "air.read_write"),
                s(m, "air.address_space"), Some(v1), s(m, "air.arg_type_size"), Some(v1), s(m, "air.arg_type_align_size"), Some(v1),
                s(m, "air.arg_type_name"), s(m, "char"), s(m, "air.arg_name"), s(m, name),
            ];
            m.md_node(items)
        };
        let a0 = buffer_arg(&mut self.m, 0, "mem");
        let a1 = buffer_arg(&mut self.m, 1, "params");
        let a2 = {
            let items = vec![Some(v2), s(&mut self.m, "air.thread_position_in_grid"), s(&mut self.m, "air.arg_type_name"), s(&mut self.m, "uint"), s(&mut self.m, "air.arg_name"), s(&mut self.m, "id")];
            self.m.md_node(items)
        };
        let mut all = vec![Some(a0), Some(a1), Some(a2)];
        if self.kernel_arity == 5 {
            let (v3, v4) = (c(&mut self.m, 3), c(&mut self.m, 4));
            let a3 = {
                let items = vec![Some(v3), s(&mut self.m, "air.thread_position_in_threadgroup"), s(&mut self.m, "air.arg_type_name"), s(&mut self.m, "uint"), s(&mut self.m, "air.arg_name"), s(&mut self.m, "lane")];
                self.m.md_node(items)
            };
            let a4 = {
                let items = vec![Some(v4), s(&mut self.m, "air.threadgroup_position_in_grid"), s(&mut self.m, "air.arg_type_name"), s(&mut self.m, "uint"), s(&mut self.m, "air.arg_name"), s(&mut self.m, "group")];
                self.m.md_node(items)
            };
            all.extend([Some(a3), Some(a4)]);
        }
        let args = self.m.md_node(all);
        let kernel = self.m.md_node(vec![Some(fn_md), Some(empty), Some(args)]);
        self.m.md_named("air.kernel", vec![kernel]);
        let version = self.m.md_node(vec![Some(v2), Some(v8), Some(v0)]);
        self.m.md_named("air.version", vec![version]);
        // what Apple's compiler is told: no fast math (the IR's floats
        // are IEEE), denormals kept — where their front end always
        // writes denorms_disable, TestFloat on the GPU says this works
        let opts: Vec<usize> = ["air.compile.denorms_enable", "air.compile.fast_math_disable", "air.compile.framebuffer_fetch_enable"]
            .iter()
            .map(|o| {
                let st = self.m.md_str(o);
                self.m.md_node(vec![Some(st)])
            })
            .collect();
        self.m.md_named("air.compile_options", opts);
        let metal = self.m.md_str("Metal");
        let v4 = c(&mut self.m, 4);
        let lang = self.m.md_node(vec![Some(metal), Some(v4), Some(v0), Some(v0)]);
        self.m.md_named("air.language_version", vec![lang]);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    /// examples/reduce.ssa on this Mac's GPU: 256 threads in groups of
    /// 64, each group summing its ids through threadgroup memory
    #[test]
    fn reduction_runs_on_the_gpu() {
        let _turn = crate::suite::tests::boot_turn();
        let platform = crate::platform::Platform::load("air").unwrap();
        let policy = platform.adjust(crate::ssa::Policy::new(crate::ssa::Type::I64).unwrap());
        let src = std::fs::read_to_string("examples/reduce.ssa").unwrap();
        let mut module = crate::ssa::parse_with(&crate::ssa::with_prelude(&src), &policy).unwrap();
        crate::ssa::resolve_types(&mut module, &policy);
        crate::ssa::verify(&module).unwrap();
        crate::opt::optimize(&mut module, crate::opt::MAX_LEVEL);
        let c = super::compile_with(&module, &platform).unwrap();
        assert!(c.has_kernel);
        let dir = std::env::temp_dir().join("probe-air-test");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("reduce.metallib"), &c.metallib).unwrap();
        let data_hex: String = c.layout.data.iter().map(|b| format!("{:02x}", b)).collect();
        std::fs::write(dir.join("reduce.air.json"), format!("{{\"data\":\"{}\",\"slab\":{},\"kernel\":true}}", data_hex, c.layout.slab)).unwrap();
        let out = std::process::Command::new("python3")
            .args(["tools/driver_metal.py", "--kernel"])
            .arg(dir.join("reduce.metallib"))
            .arg(dir.join("reduce.air.json"))
            .args(["256", "64"])
            .output()
            .unwrap();
        let text = String::from_utf8_lossy(&out.stdout);
        assert!(text.starts_with("area: [2016, 6112, 10208, 14304, 0,"), "{}{}", text, String::from_utf8_lossy(&out.stderr));
    }
}
