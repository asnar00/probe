//! The incremental JIT arena: all compiled functions live in one block of
//! executable memory, each in its own slot with slack, so a changed
//! definition can be recompiled *in place* — or, if it outgrew its slot,
//! relocated to the arena tail — without touching anything else.
//!
//! What makes relocation and re-optimization free is that **call sites are
//! never patched**: every call goes through a fixed per-function
//! trampoline. The trampoline (built from learned templates like all our
//! code) bumps an invocation counter and branches to the function's
//! current address; replacing a function means rewriting one trampoline's
//! target. The counters give the system real hotness data — recursion and
//! internal calls included — so it can decide when a tier-0 function has
//! earned a trip through the optimization pipeline.
//!
//! Layout:  [tramp f][tramp g]... [code f + slack][code g + slack]...
//! A grown function abandons its old slot for a fresh one at the tail;
//! the trampoline retarget makes the move invisible.

use crate::emit::{compile_one, Encoder};
use crate::ssa::Function;
use std::collections::HashMap;

unsafe extern "C" {
    fn mmap(addr: *mut u8, len: usize, prot: i32, flags: i32, fd: i32, offset: i64) -> *mut u8;
    fn pthread_jit_write_protect_np(enabled: i32);
    fn sys_icache_invalidate(start: *mut u8, len: usize);
}

const ARENA_SIZE: usize = 16 << 20;
const MAX_FUNCS: usize = 1024;
const TRAMP_SIZE: usize = 64; // 14 instructions used; padded
/// slot capacity: code size plus half again, plus headroom, 16-aligned
fn slack(len: usize) -> usize {
    (len + len / 2 + 64 + 15) & !15
}

struct Entry {
    tramp: usize,   // arena offset of the trampoline (never moves)
    counter: usize, // index into the counters slab
    slot: usize,    // arena offset of current code
    cap: usize,     // slot capacity
    level: usize,   // pipeline level this code was compiled at
    source: String, // pretty-printed SSA, for change detection
}

pub struct Arena {
    base: *mut u8,
    cursor: usize,
    entries: HashMap<String, Entry>,
    counters: Box<[std::cell::UnsafeCell<u64>; MAX_FUNCS]>,
    n_funcs: usize,
    enc: Encoder,
    /// callee name -> native op, per the platform; set when a module loads
    pub natives: HashMap<String, crate::platform::Native>,
}

pub struct Installed {
    pub name: String,
    pub level: usize,
    pub bytes: usize,
    pub in_place: bool,
}

impl Arena {
    pub fn new(enc: Encoder) -> Result<Arena, String> {
        let base = unsafe {
            mmap(
                std::ptr::null_mut(),
                ARENA_SIZE,
                1 | 2 | 4,             // read | write | exec
                0x0002 | 0x1000 | 0x0800, // private | anon | jit
                -1,
                0,
            )
        };
        if base as isize == -1 {
            return Err("mmap(MAP_JIT) failed".into());
        }
        Ok(Arena {
            base,
            cursor: 0,
            entries: HashMap::new(),
            counters: Box::new([const { std::cell::UnsafeCell::new(0) }; MAX_FUNCS]),
            n_funcs: 0,
            enc,
            natives: HashMap::new(),
        })
    }

    fn alloc(&mut self, size: usize) -> Result<usize, String> {
        let off = (self.cursor + 15) & !15;
        if off + size > ARENA_SIZE {
            return Err("jit arena is full".into());
        }
        self.cursor = off + size;
        Ok(off)
    }

    fn write(&self, off: usize, bytes: &[u8]) {
        unsafe {
            pthread_jit_write_protect_np(0);
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), self.base.add(off), bytes.len());
            pthread_jit_write_protect_np(1);
            sys_icache_invalidate(self.base.add(off), bytes.len());
        }
    }

    /// movz/movk a 64-bit absolute value into register `reg`
    fn mat64(&self, reg: i64, v: u64, out: &mut Vec<u8>) -> Result<(), String> {
        let t = [
            "movz {x}, #{i 0..65535}",
            "movk {x}, #{i 0..65535}, lsl #16",
            "movk {x}, #{i 0..65535}, lsl #32",
            "movk {x}, #{i 0..65535}, lsl #48",
        ];
        for (i, tpl) in t.iter().enumerate() {
            let chunk = ((v >> (16 * i)) & 0xffff) as i64;
            out.extend(self.enc.encode(tpl, &[reg, chunk])?.to_le_bytes());
        }
        Ok(())
    }

    /// the trampoline body: count the call, jump to the current code.
    /// x16/x17 are the platform's intra-call scratch registers, safe here.
    fn tramp_code(&self, counter_addr: u64, target_addr: u64) -> Result<Vec<u8>, String> {
        let mut b = Vec::new();
        self.mat64(17, counter_addr, &mut b)?;
        b.extend(
            self.enc
                .encode("ldr {x}, [{x}, #{i 0..32760 /8}]", &[16, 17, 0])?
                .to_le_bytes(),
        );
        b.extend(
            self.enc
                .encode("add {x}, {x}, #{i 0..4095}", &[16, 16, 1])?
                .to_le_bytes(),
        );
        b.extend(
            self.enc
                .encode("str {x}, [{x}, #{i 0..32760 /8}]", &[16, 17, 0])?
                .to_le_bytes(),
        );
        self.mat64(17, target_addr, &mut b)?;
        b.extend(self.enc.encode("br {x}", &[17])?.to_le_bytes());
        Ok(b)
    }

    fn retarget(&self, name: &str) -> Result<(), String> {
        let e = &self.entries[name];
        let counter_addr = self.counters[e.counter].get() as u64;
        let target_addr = self.base as u64 + e.slot as u64;
        let code = self.tramp_code(counter_addr, target_addr)?;
        self.write(e.tramp, &code);
        Ok(())
    }

    /// Make sure every named function has a trampoline (created before any
    /// compilation so mutual recursion resolves) at a fixed arena offset.
    fn ensure_trampolines(&mut self, names: &[&str]) -> Result<(), String> {
        for &name in names {
            if self.entries.contains_key(name) {
                continue;
            }
            if self.n_funcs == MAX_FUNCS {
                return Err("too many functions for the arena".into());
            }
            let tramp = self.alloc(TRAMP_SIZE)?;
            self.entries.insert(
                name.to_string(),
                Entry {
                    tramp,
                    counter: self.n_funcs,
                    slot: usize::MAX,
                    cap: 0,
                    level: usize::MAX,
                    source: String::new(),
                },
            );
            self.n_funcs += 1;
        }
        Ok(())
    }

    /// Compile `func` at `level` and install it: in place when it fits its
    /// slot, relocated to the tail when it doesn't. `func` is the raw
    /// (unoptimized) SSA; the pipeline prefix runs here.
    pub fn install(&mut self, func: &Function, level: usize) -> Result<Installed, String> {
        self.ensure_trampolines(&[func.name.as_str()])?;
        let source = func.to_string();

        let mut opt_func = func.clone();
        crate::opt::optimize_function(&mut opt_func, level);

        // calls resolve to callee trampolines, so nothing here ever needs
        // re-patching when someone else moves
        let tramps: HashMap<String, i64> = self
            .entries
            .iter()
            .map(|(n, e)| (n.clone(), e.tramp as i64))
            .collect();
        let resolve = move |callee: &str| tramps.get(callee).copied();
        let (old_slot, old_cap) = {
            let e = &self.entries[&func.name];
            (e.slot, e.cap)
        };
        // bl is pc-relative, so code depends on where it will live: compile
        // once at the likely base and only recompile if placement changes
        let tail = (self.cursor + 15) & !15;
        let trial_base = if old_slot != usize::MAX { old_slot } else { tail };
        let code = compile_one(&opt_func, &self.enc, &self.natives, trial_base as i64, &resolve)?;

        let (slot, cap, in_place) = if old_slot != usize::MAX && code.len() <= old_cap {
            (old_slot, old_cap, true)
        } else {
            let cap = slack(code.len());
            (self.alloc(cap)?, cap, false)
        };
        let code = if slot == trial_base {
            code
        } else {
            compile_one(&opt_func, &self.enc, &self.natives, slot as i64, &resolve)?
        };
        self.write(slot, &code);

        let e = self.entries.get_mut(&func.name).unwrap();
        e.slot = slot;
        e.cap = cap;
        e.level = level;
        e.source = source;
        self.retarget(&func.name)?;
        Ok(Installed {
            name: func.name.clone(),
            level,
            bytes: code.len(),
            in_place,
        })
    }

    /// Install every function in the module that is new or whose source
    /// changed, at the given level. Returns what was (re)compiled.
    pub fn sync(&mut self, funcs: &[Function], level: usize) -> Result<Vec<Installed>, String> {
        let names: Vec<&str> = funcs.iter().map(|f| f.name.as_str()).collect();
        self.ensure_trampolines(&names)?;
        let mut out = Vec::new();
        for func in funcs {
            let changed = match self.entries.get(&func.name) {
                Some(e) => e.slot == usize::MAX || e.source != func.to_string(),
                None => true,
            };
            if changed {
                out.push(self.install(func, level)?);
            }
        }
        Ok(out)
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub fn calls(&self, name: &str) -> u64 {
        self.entries
            .get(name)
            .map(|e| unsafe { std::ptr::read_volatile(self.counters[e.counter].get()) })
            .unwrap_or(0)
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub fn level(&self, name: &str) -> Option<usize> {
        self.entries.get(name).map(|e| e.level)
    }

    /// Names of installed functions, hottest first.
    pub fn by_heat(&self) -> Vec<(String, u64, usize)> {
        let mut v: Vec<(String, u64, usize)> = self
            .entries
            .iter()
            .filter(|(_, e)| e.slot != usize::MAX)
            .map(|(n, e)| {
                let c = unsafe { std::ptr::read_volatile(self.counters[e.counter].get()) };
                (n.clone(), c, e.level)
            })
            .collect();
        v.sort_by(|a, b| b.1.cmp(&a.1));
        v
    }

    fn entry_ptr(&self, name: &str) -> Result<*const u8, String> {
        let e = self
            .entries
            .get(name)
            .filter(|e| e.slot != usize::MAX)
            .ok_or_else(|| format!("no function {} installed", name))?;
        // call through the trampoline so the invocation counts
        Ok(unsafe { self.base.add(e.tramp) })
    }

    pub fn call(&self, name: &str, args: &[i64]) -> Result<i64, String> {
        let p = self.entry_ptr(name)?;
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

    pub fn call2(&self, name: &str, args: &[i64]) -> Result<(i64, i64), String> {
        #[repr(C)]
        struct Pair(i64, i64);
        let p = self.entry_ptr(name)?;
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
            n => return Err(format!("{} arguments not supported", n)),
        };
        Ok((r.0, r.1))
    }
}

// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(src: &str) -> crate::ssa::Module {
        let m = crate::ssa::parse(src).expect("parse");
        crate::ssa::verify(&m).expect("verify");
        m
    }

    fn arena() -> Arena {
        let enc = Encoder::load("targets/arm64.encodings.json").expect("encodings");
        Arena::new(enc).expect("arena")
    }

    #[test]
    fn incremental_replace_and_grow() {
        let mut a = arena();
        let m1 = parse(
            "fn val() -> i64 {\ne:\n    v: i64 = const 1\n    ret v\n}\n\
             fn twice() -> i64 {\ne:\n    a: i64 = val()\n    b: i64 = add a, a\n    ret b\n}\n",
        );
        a.sync(&m1.funcs, 0).expect("install");
        assert_eq!(a.call("twice", &[]).unwrap(), 2);
        assert!(a.calls("val") >= 1, "trampoline counter must tick");

        // small edit: recompiles in place, twice untouched but sees it
        let m2 = parse(
            "fn val() -> i64 {\ne:\n    v: i64 = const 21\n    ret v\n}\n\
             fn twice() -> i64 {\ne:\n    a: i64 = val()\n    b: i64 = add a, a\n    ret b\n}\n",
        );
        let done = a.sync(&m2.funcs, 0).expect("sync");
        assert_eq!(done.len(), 1, "only val changed");
        assert!(done[0].in_place, "same-size edit must reuse the slot");
        assert_eq!(a.call("twice", &[]).unwrap(), 42);

        // big edit: outgrows the slot, relocates, trampoline hides the move
        // a long straight-line chain (level 0 folds nothing, so it all emits)
        let mut big = String::from("fn val() -> i64 {\ne:\n    v0: i64 = const 2\n");
        for i in 1..=20 {
            big.push_str(&format!("    v{}: i64 = add v{}, v{}\n", i, i - 1, i - 1));
        }
        big.push_str("    ten: i64 = const 1048576\n    r: i64 = div v20, ten\n    ret r\n}\n");
        // 2^21 / 2^20 = 2 -> twice = 4... keep the arithmetic honest below
        let m3 = parse(&format!(
            "{}fn twice() -> i64 {{\ne:\n    a: i64 = val()\n    b: i64 = add a, a\n    ret b\n}}\n",
            big
        ));
        let done = a.sync(&m3.funcs, 0).expect("sync big");
        assert!(!done[0].in_place, "bigger body must relocate");
        assert_eq!(a.call("twice", &[]).unwrap(), 4); // (2 << 20 >> 20) * 2

        // promotion: recompile hot val at the top level, same trampoline
        let val = m3.funcs.iter().find(|f| f.name == "val").unwrap();
        let promoted = a.install(val, crate::opt::MAX_LEVEL).expect("promote");
        assert_eq!(promoted.level, crate::opt::MAX_LEVEL);
        // at max level the chain folds to a constant; result is unchanged
        assert_eq!(a.call("twice", &[]).unwrap(), 4);
        assert_eq!(a.level("val"), Some(crate::opt::MAX_LEVEL));
    }

    #[test]
    fn recursion_counts_and_survives_promotion() {
        let mut a = arena();
        let src = std::fs::read_to_string("examples/fib.ssa").unwrap();
        let m = parse(&src);
        a.sync(&m.funcs, 0).expect("install");
        assert_eq!(a.call("fib", &[15]).unwrap(), 610);
        let calls = a.calls("fib");
        assert!(calls > 500, "recursive calls must count, got {}", calls);
        a.install(&m.funcs[0], crate::opt::MAX_LEVEL).expect("promote");
        assert_eq!(a.call("fib", &[15]).unwrap(), 610);
    }
}
