//! Three places over wires that take time (hop fifty-nine; fm3
//! places.md, "two pushes at once: the line is the merge", "whose clock
//! stamps a shared item" and "losing a wire or a place"). Each person's
//! device is a client of its own and runs every line of theirs, as the
//! rule of "whose is not where" has it for a line that reads the
//! device's own streams; the server is the home of every group's and
//! everyone's stream; a wire goes from each device to the server and
//! none between two devices. A push into a shared stream is shown at
//! the device at once, against its copy, and **what crosses is the
//! push**: the device's turn as it began, which the home runs again
//! against its own values in the order the pushes reach it, its text
//! written nowhere (Bayou's tentative write run again in the committed
//! order; Replicache's mutation run on the client and on the server).
//! The home's values then cross back to every copy, which takes them
//! and runs again on top whatever of its own the home had not had yet.
//!
//! This file rewrites the text of a store built for people at one place
//! (`lower.rs`, hop fifty-seven), as `places.rs` rewrites a placed
//! build: the copies are no longer written back and taken where the
//! current device changes, a device's input turn and a case's call on it
//! are kept and sent, and the list of things going on has the wire's
//! arrivals in it. Each wire has a delay and may be gone for a while;
//! the server may be gone, which is every wire gone at once, its values
//! as they were when it comes back; a device may be gone, its context
//! frozen, no key taken and nothing shown, and its copy catches up from
//! the home when it comes back.

use super::turns::parse;

/// a context's own region of the arena, as one turn kept for running
/// again holds it, in bytes; and how many turns may be kept at once
const REGION: usize = 8192;
const SNAPS: usize = 16;
/// the wire's arrivals in flight, the home's values each carries, and
/// the record's lines
const EVENTS: usize = 256;
const PAYLOADS: usize = 128;
const LOG: usize = 512;
const NEVER: i64 = 1 << 62;
/// how many of the pushes it has had the home keeps, to put one that
/// arrives late but sorts before them in its order
const KEEP: usize = 8;

/// How the runner says the wires: a delay for each person's wire, and
/// times when a wire, the server or a device is gone. `who` is a
/// person's wire (0 to 3), the server (4), or a person's device (8 + k)
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Wires {
    pub delay: Vec<i64>,
    pub gone: Vec<(i64, i64, i64)>,
}

pub const SERVER: i64 = 4;
pub const DEVICE: i64 = 8;

impl Wires {
    /// the calls that say them, made before the case
    pub fn calls(&self) -> Vec<(String, Vec<i64>)> {
        let mut out = Vec::new();
        for k in 0..4 {
            let d = self.delay.get(k).copied().or(self.delay.last().copied()).unwrap_or(0);
            out.push(("__hw_set".to_string(), vec![1 + k as i64, d]));
        }
        out.push(("__hw_set".to_string(), vec![7, self.gone.len() as i64]));
        for (i, (who, from, to)) in self.gone.iter().enumerate() {
            out.push(("__hw_set".to_string(), vec![8 + 3 * i as i64, *who]));
            out.push(("__hw_set".to_string(), vec![9 + 3 * i as i64, *from]));
            out.push(("__hw_set".to_string(), vec![10 + 3 * i as i64, *to]));
        }
        out
    }
}

fn fields_of(lines: &[String], ty: &str) -> Vec<(String, String)> {
    let head = format!("type {} = struct", ty);
    let Some(at) = lines.iter().position(|l| *l == head) else { return Vec::new() };
    lines[at + 1..].iter().take_while(|l| l.starts_with("    ")).filter_map(|l| l.trim().split_once(": ")).map(|(n, t)| (n.to_string(), t.to_string())).collect()
}

fn word(ty: &str) -> bool {
    matches!(ty, "int" | "i64" | "i32" | "i16" | "i8" | "u64" | "u32" | "u16" | "u8" | "u1" | "index")
}

/// The rewrite: the text of the store at three places over wires that
/// take time, or why it cannot be had yet
pub fn rewrite(ir: &str) -> Result<String, String> {
    let (lines, fns) = parse(ir);
    if !lines.iter().any(|l| l.starts_with("data __share: ")) {
        return Err("the store is not built for people".into());
    }
    let group = fields_of(&lines, "__group");
    let global = fields_of(&lines, "__global");
    if let Some((n, _)) = group.iter().chain(global.iter()).find(|(_, t)| t.ends_with('$')) {
        return Err(format!("'{}$' is a stored stream that is shared, and a stored stream's copies over a wire that takes time are not built yet: a value kept as one word is (hop fifty-nine)", n));
    }
    if let Some((n, _)) = group.iter().chain(global.iter()).find(|(_, t)| !word(t)) {
        return Err(format!("'{}$' is shared and wider than a word, which a wire that takes time does not carry yet", n));
    }
    let turn_here = fns.iter().find(|f| f.name == "__turn_here").ok_or("the store has no input that arrives at a time, and nothing to cross a wire")?;
    let simple = ["_this: ptr = context()", "x: __ctx = load _this", "d1: i64 = get x, __due_in", "__wait(d1)", "__in_turn()", "ret"];
    let body: Vec<&str> = turn_here.body.iter().map(|l| l.trim()).collect();
    if body != simple {
        return Err("the store's things going on are more than each device's input: a clock or a slot's close crossing a wire that takes time is not built yet".into());
    }
    // the twins of a case's call: each person's call, kept and sent
    let twins: Vec<(String, String)> = fns.iter().filter_map(|f| f.name.strip_prefix("__people_").map(|g| (f.name.clone(), g.to_string()))).collect();
    for (t, _) in &twins {
        let f = fns.iter().find(|f| &f.name == t).unwrap();
        if !f.params.is_empty() {
            return Err(format!("'{}' is handed numbers, and a case's call with numbers crossing a wire that takes time is not built yet", t));
        }
    }
    let gp = !group.is_empty();
    let lp = !global.is_empty();
    let mut out = String::new();
    let mut k = 0;
    while k < lines.len() {
        let Some(f) = fns.iter().find(|f| f.at.0 == k) else {
            out.push_str(&lines[k]);
            out.push('\n');
            if lines[k].starts_with("data __share: ") {
                out.push_str(&format!("; (hop fifty-nine) three places over wires that take time: `__hw` is the wires' words, 0 how the runner reads back (2 the record), 1 to 4 each person's delay, 7 how many times something is gone and from 8 each as who (a person's wire, 4 the server, 8 and on a person's device), from and to; 40 how far the arrivals reach, 41 the record's length, 42 how many of the home's values have been kept, 44 to 47 how many pushes each person has made, 48 to 51 how many the home has had of each, 52 the turn kept last. `__hw_reg` is each device's region of the arena; `__hw_sc`, `__hw_sm` and `__hw_sk` the turns kept, their context, their region and eight words each (whose, which kind, which call, the time, how much its device had written before and after, the number of its push, whether it waits on the home and on its copy); `__hw_ev` the arrivals, eight words each (when, 1 a push at the home or 2 the home's values at a copy, whose, the push's own time, its turn or the values', alive); `__hw_gv`, `__hw_lv` and `__hw_pa` the home's values sent and how much of each person's each had had; `__hw_pk` each person's turn of each push; `__hw_x` the context a turn is run again in, with a capture of its own that nobody reads\ndata __hw: array(i64, 64)\ndata __hw_reg: array(i64, 8)\ndata __hw_sc: array(__ctx, {s})\ndata __hw_sm: array(u8, {sm})\ndata __hw_sk: array(i64, {sk})\ndata __hw_ev: array(i64, {ev})\ndata __hw_pa: array(i64, {pa})\ndata __hw_pk: array(i64, 256)\ndata __hw_x: array(__ctx, 1)\ndata __hw_cap: array(u8, 8192)\ndata __hw_capm: array(i64, 8193)\ndata __hw_save: array(u8, {r})\ndata __hw_log: array(i64, {log})\ndata __hw_ho: array(i64, {s})\ndata __hw_m: array(i64, 64)\ndata __hw_pm: array(i64, {pm})\n{g}{l}", s = SNAPS, sm = SNAPS * REGION, sk = SNAPS * 8, ev = EVENTS * 8, pa = PAYLOADS * 4, r = REGION, log = LOG * 6, pm = PAYLOADS * 8, g = if gp { format!("data __hw_gv: array(__group, {})\ndata __hw_tg: array(__group, 1)\ndata __hw_bg: array(__group, 1)\n", PAYLOADS) } else { String::new() }, l = if lp { format!("data __hw_lv: array(__global, {})\ndata __hw_tl: array(__global, 1)\ndata __hw_bl: array(__global, 1)\n", PAYLOADS) } else { String::new() }));
            }
            k += 1;
            continue;
        };
        let end = f.at.1;
        match f.name.as_str() {
            "__zero_context" => {
                out.push_str("; the runner's: the k-th device is the current one. Over a wire that takes time (hop fifty-nine) its copies of the shared streams are its own, and the wire alone changes them\nfn __zero_context(k: i32)\n    sp: ptr = addr __share\n    p: ptr(array(__ctx, 4)) = addr __ctx_mem\n    i: index = conv k\n    e: ptr(__ctx) = index p, i\n    c: ptr = cast e\n    context_set(c)\n    kw: i64 = conv k\n    store kw, sp, 8\n    ret\n");
            }
            "__turn_here" => {
                out.push_str("; (hop fifty-nine) this device's earliest turn, which the caller knows is due: where the device is gone its key is lost and nothing runs; otherwise the turn is kept as it begins, taken, and sent to the home as the push it is\nfn __turn_here()\n    _this: ptr = context()\n    x: __ctx = load _this\n    d1: i64 = get x, __due_in\n    __wait(d1)\n    sp: ptr = addr __share\n    k: i64 = load sp, 8\n    off: u1 = __hw_off(k, d1)\n    if off\n        __hw_skip()\n        ret\n    s: i64 = __hw_snap(k, 1, d1)\n    __in_turn()\n    __hw_send(k, s, d1)\n    ret\n");
            }
            "__out_len" => {
                out.push_str("fn __out_len() -> i64\n    hm: ptr = addr __hw\n    hmode: i64 = load hm\n    hrec: u1 = cmp.eq hmode, 2\n    if hrec\n        hn: i64 = load hm, 328\n        h6: i64 = mul hn, 6\n        ret h6\n");
                for l in &lines[f.at.0 + 1..end] {
                    out.push_str(l);
                    out.push('\n');
                }
            }
            "__out_mark" => {
                let i = f.params.first().map(|(n, _)| n.clone()).unwrap_or_else(|| "i".into());
                out.push_str(&format!("{}\n    hm: ptr = addr __hw\n    hmode: i64 = load hm\n    hrec: u1 = cmp.eq hmode, 2\n    if hrec\n        hl: ptr = addr __hw_log\n        hv: i64 = load hl, {}, 8\n        ret hv\n", lines[f.at.0], i));
                for l in &lines[f.at.0 + 1..end] {
                    out.push_str(l);
                    out.push('\n');
                }
            }
            "__zero_reset" => {
                // the wires' words emptied with the store, before the runner says them
                out.push_str(&lines[f.at.0]);
                out.push('\n');
                out.push_str(&format!("    hz: ptr = addr __hw\n    loop(hi: index = 0) bound 64\n        hd: u1 = cmp.ge hi, 64\n        if hd\n            break\n        store 0: i64, hz, hi, 8\n        hj: index = add hi, 1\n        continue hj\n    hmz: ptr = addr __hw_m\n    loop(mzi: index = 0) bound 64\n        mzd: u1 = cmp.ge mzi, 64\n        if mzd\n            break\n        store 0: i64, hmz, mzi, 8\n        mzj: index = add mzi, 1\n        continue mzj\n    hk: ptr = addr __hw_sk\n    loop(si: index = 0) bound {sk}\n        sd: u1 = cmp.ge si, {sk}\n        if sd\n            break\n        store 0: i64, hk, si, 8\n        sj: index = add si, 1\n        continue sj\n    he: ptr = addr __hw_ev\n    loop(ei: index = 0) bound {ev}\n        ed: u1 = cmp.ge ei, {ev}\n        if ed\n            break\n        e8: index = mul ei, 8\n        e5: index = add e8, 5\n        store 0: i64, he, e5, 8\n        ej: index = add ei, 1\n        continue ej\n", sk = SNAPS * 8, ev = EVENTS));
                for l in &lines[f.at.0 + 1..end] {
                    out.push_str(l);
                    out.push('\n');
                }
            }
            "__zero_new" => {
                // each device's region of the arena, from where it begins to where it ends
                out.push_str(&lines[f.at.0]);
                out.push('\n');
                out.push_str("    hab: ptr = addr __arena\n    ha0: index = load hab, 16\n");
                for l in &lines[f.at.0 + 1..end] {
                    if l.trim() == "ret" {
                        out.push_str("    ha1: index = load hab, 16\n    hks: ptr = addr __share\n    hkw: i64 = load hks, 8\n    hk2: i64 = mul hkw, 2\n    hki: index = conv hk2\n    hr: ptr = addr __hw_reg\n    ha0w: i64 = conv ha0\n    store ha0w, hr, hki, 8\n    hlen: index = sub ha1, ha0\n    hfit: u1 = cmp.le hlen, 8192\n    check hfit\n    hlw: i64 = conv hlen\n    hki1: index = add hki, 1\n    store hlw, hr, hki1, 8\n");
                    }
                    out.push_str(l);
                    out.push('\n');
                }
            }
            "__turns" => {
                // the wire's arrivals are things going on: one due no later than the earliest turn of a device goes first
                for l in &lines[f.at.0..end] {
                    if l.trim() == "none: u1 = cmp.lt bk, 0" {
                        let p = &l[..l.len() - l.trim_start().len()];
                        out.push_str(&format!("{p}hd: i64 = __hw_soon(t)\n{p}hb: i64 = load sp, 16\n{p}hfirst: u1 = cmp.le hd, hb\n{p}hnv: u1 = cmp.lt hd, {never}\n{p}hgo: u1 = and hfirst, hnv\n{p}if hgo\n{p}    __hw_take()\n{p}    nh: i64 = add n, 1\n{p}    continue nh\n", p = p, never = NEVER));
                    }
                    out.push_str(l);
                    out.push('\n');
                }
            }
            name if twins.iter().any(|(t, _)| t == name) => {
                // each person's call kept as it begins and sent; at the end the record says each shared stream at the home and at each copy
                let g = &twins.iter().find(|(t, _)| t == name).unwrap().1;
                let id = twins.iter().position(|(t, _)| t == name).unwrap() + 1;
                for l in &lines[f.at.0..end] {
                    let t = l.trim();
                    if t == format!("{}()", g) {
                        let p = &l[..l.len() - l.trim_start().len()];
                        out.push_str(&format!("{p}hs: i64 = __hw_snap(k, {kind}, 0)\n{p}{g}()\n{p}__hw_send(k, hs, 0)\n", p = p, kind = 1 + id, g = g));
                        continue;
                    }
                    if t == "__gather()" {
                        out.push_str("    __hw_end()\n");
                    }
                    out.push_str(l);
                    out.push('\n');
                }
            }
            _ => {
                for l in &lines[f.at.0..end] {
                    out.push_str(l);
                    out.push('\n');
                }
            }
        }
        k = end;
    }
    out.push_str(&functions(&group, &global, &twins));
    Ok(out)
}

/// the functions of the wires and the home
fn functions(group: &[(String, String)], global: &[(String, String)], twins: &[(String, String)]) -> String {
    let gp = !group.is_empty();
    let lp = !global.is_empty();
    let mut t = String::new();
    t.push_str("\n; (hop fifty-nine) the runner's: a word of the wires\nfn __hw_set(i: index, v: i64)\n    p: ptr = addr __hw\n    store v, p, i, 8\n    ret\n");
    // when the wire of person k is next up, from t: its own, the server's and the device's gone times
    t.push_str("\n; (hop fifty-nine) the first moment from t that person k's wire is up: neither it, nor the server, nor the device gone then\nfn __hw_up(k: i64, t: i64) -> i64\n    p: ptr = addr __hw\n    n: i64 = load p, 56\n    dk: i64 = add k, 8\n    r: i64 = loop(pass: i64 = 0, u: i64 = t)\n        pd: u1 = cmp.ge pass, 9\n        if pd\n            break u\n        u2: i64 = loop(i: i64 = 0, v: i64 = u)\n            id: u1 = cmp.ge i, n\n            if id\n                break v\n            b: i64 = mul i, 3\n            b8: i64 = add b, 8\n            bi: index = conv b8\n            who: i64 = load p, bi, 8\n            b9: index = add bi, 1\n            from: i64 = load p, b9, 8\n            b10: index = add bi, 2\n            to: i64 = load p, b10, 8\n            w0: u1 = cmp.eq who, k\n            w1: u1 = cmp.eq who, 4\n            w2: u1 = cmp.eq who, dk\n            w01: u1 = or w0, w1\n            mine: u1 = or w01, w2\n            a: u1 = cmp.le from, v\n            z: u1 = cmp.lt v, to\n            az: u1 = and a, z\n            hit: u1 = and mine, az\n            v2: i64 = if hit\n                yield to\n            else\n                yield v\n            i2: i64 = add i, 1\n            continue i2, v2\n        pass2: i64 = add pass, 1\n        continue pass2, u2\n    ret r\n");
    t.push_str("\n; (hop fifty-nine) is person k's device gone at t\nfn __hw_off(k: i64, t: i64) -> u1\n    p: ptr = addr __hw\n    n: i64 = load p, 56\n    dk: i64 = add k, 8\n    r: u1 = loop(i: i64 = 0)\n        id: u1 = cmp.ge i, n\n        if id\n            break 0\n        b: i64 = mul i, 3\n        b8: i64 = add b, 8\n        bi: index = conv b8\n        who: i64 = load p, bi, 8\n        b9: index = add bi, 1\n        from: i64 = load p, b9, 8\n        b10: index = add bi, 2\n        to: i64 = load p, b10, 8\n        w2: u1 = cmp.eq who, dk\n        a: u1 = cmp.le from, t\n        z: u1 = cmp.lt t, to\n        az: u1 = and a, z\n        hit: u1 = and w2, az\n        if hit\n            break 1\n        i2: i64 = add i, 1\n        continue i2\n    ret r\n");
    // a key lost to a device that is gone
    t.push_str("\n; (hop fifty-nine) the key due at a device that is gone: lost, the next one due\nfn __hw_skip()\n    _this: ptr = context()\n    x: __ctx = load _this\n    k: index = get x, __in_k\n    n: index = get x, __in_n\n    tp: ptr = get x, __in_t\n    k2: index = add k, 1\n    more: u1 = cmp.lt k2, n\n    d: i64 = if more\n        t: i64 = load tp, k2, 8\n        yield t\n    else\n        never: i64 = const 4611686018427387904\n        yield never\n    y: __ctx = load _this\n    y2: __ctx = set y, __in_k, k2\n    y3: __ctx = set y2, __due_in, d\n    store y3, _this\n    ret\n");
    // a line of the record
    t.push_str(&format!("\n; (hop fifty-nine) a line of the record: what it is and five words\nfn __hw_note(kind: i64, a: i64, b: i64, c: i64, d: i64, e: i64)\n    p: ptr = addr __hw\n    n: i64 = load p, 328\n    room: u1 = cmp.lt n, {log}\n    if room\n    else\n        ret\n    l: ptr = addr __hw_log\n    b0: i64 = mul n, 6\n    i0: index = conv b0\n    store kind, l, i0, 8\n    i1: index = add i0, 1\n    store a, l, i1, 8\n    i2: index = add i0, 2\n    store b, l, i2, 8\n    i3: index = add i0, 3\n    store c, l, i3, 8\n    i4: index = add i0, 4\n    store d, l, i4, 8\n    i5: index = add i0, 5\n    store e, l, i5, 8\n    n2: i64 = add n, 1\n    store n2, p, 328\n    ret\n", log = LOG));
    // the copies in and out of a context, from and to the given structs
    let mut inn = String::from("\n; (hop fifty-nine) the current context's copies of the group's and everyone's streams set from the values at gp and lp\nfn __hw_in(gp: ptr, lp: ptr)\n    c: ptr = context()\n");
    let mut outt = String::from("\n; (hop fifty-nine) ... and the values at gp and lp set from the current context's copies\nfn __hw_out(gp: ptr, lp: ptr)\n    c: ptr = context()\n    x: __ctx = load c\n");
    if gp {
        inn.push_str("    sg: __group = load gp\n");
        outt.push_str("    sg0: __group = load gp\n");
    }
    if lp {
        inn.push_str("    sl: __global = load lp\n");
        outt.push_str("    sl0: __global = load lp\n");
    }
    inn.push_str("    y0: __ctx = load c\n");
    let mut y = 0;
    let (mut g, mut l) = (0, 0);
    for (kind, fs) in [("g", group), ("l", global)] {
        for (f, ty) in fs {
            inn.push_str(&format!("    v{y}: {ty} = get s{kind}, {f}\n    y{y2}: __ctx = set y{y}, {f}, v{y}\n", y = y, y2 = y + 1, ty = ty, f = f, kind = kind));
            y += 1;
            let (cur, nxt) = if kind == "g" { g += 1; (g - 1, g) } else { l += 1; (l - 1, l) };
            outt.push_str(&format!("    o{kind}{nxt}v: {ty} = get x, {f}\n    s{kind}{nxt}: __{st} = set s{kind}{cur}, {f}, o{kind}{nxt}v\n", kind = kind, nxt = nxt, cur = cur, ty = ty, f = f, st = if kind == "g" { "group" } else { "global" }));
        }
    }
    inn.push_str(&format!("    store y{}, c\n    ret\n", y));
    if gp {
        outt.push_str(&format!("    store sg{}, gp\n", g));
    }
    if lp {
        outt.push_str(&format!("    store sl{}, lp\n", l));
    }
    outt.push_str("    ret\n");
    t.push_str(&inn);
    t.push_str(&outt);
    // the home's values, as pointers
    t.push_str(&format!("\n; (hop fifty-nine) where the home's values are: the group's and everyone's state\nfn __hw_homes() -> (ptr, ptr)\n{g}{l}    ret hg, hl\n", g = if gp { "    hgm: ptr(array(__group, 1)) = addr __group_mem\n    z: index = const 0\n    hge: ptr(__group) = index hgm, z\n    hg: ptr = cast hge\n" } else { "    hg: ptr = const 0\n" }, l = if lp { "    hl: ptr = addr __global_mem\n" } else { "    hl: ptr = const 0\n" }));
    // keeping a turn as it begins
    t.push_str(&format!("\n; (hop fifty-nine) the turn of person k beginning now, kept: its context and its device's region of the arena, which kind it is (1 a key, 2 and on a case's call), its time, how much its device had written, and the number of its push. A place among the kept is free where it waits on neither the home nor its copy\nfn __hw_snap(k: i64, kind: i64, t: i64) -> i64\n    sk: ptr = addr __hw_sk\n    s: i64 = loop(i: i64 = 0)\n        full: u1 = cmp.lt i, {n}\n        check full\n        b: i64 = mul i, 8\n        b6: i64 = add b, 6\n        bi: index = conv b6\n        w1: i64 = load sk, bi, 8\n        b7: index = add bi, 1\n        w2: i64 = load sk, b7, 8\n        busy: i64 = or w1, w2\n        free: u1 = cmp.eq busy, 0\n        if free\n            break i\n        i2: i64 = add i, 1\n        continue i2\n    c: ptr = context()\n    x: __ctx = load c\n    scm: ptr(array(__ctx, {n})) = addr __hw_sc\n    si: index = conv s\n    se: ptr(__ctx) = index scm, si\n    sp: ptr = cast se\n    store x, sp\n    __hw_region(k, s, 1)\n    on: index = get x, __out_n\n    onw: i64 = conv on\n    hp: ptr = addr __hw\n    mi: i64 = add k, 44\n    mii: index = conv mi\n    m: i64 = load hp, mii, 8\n    b0: i64 = mul s, 8\n    i0: index = conv b0\n    store k, sk, i0, 8\n    i1: index = add i0, 1\n    store kind, sk, i1, 8\n    i3: index = add i0, 3\n    store t, sk, i3, 8\n    i4: index = add i0, 4\n    store onw, sk, i4, 8\n    i5: index = add i0, 5\n    store m, sk, i5, 8\n    i6: index = add i0, 6\n    store 1: i64, sk, i6, 8\n    i7: index = add i0, 7\n    store 1: i64, sk, i7, 8\n    km: i64 = mul k, 64\n    mm: i64 = rem m, 64\n    pki: i64 = add km, mm\n    pkx: index = conv pki\n    pk: ptr = addr __hw_pk\n    store s, pk, pkx, 8\n    m2: i64 = add m, 1\n    store m2, hp, mii, 8\n    ret s\n", n = SNAPS));
    // a device's region copied into a kept turn, or back
    t.push_str(&format!("\n; (hop fifty-nine) device k's region of the arena copied into kept turn s (dir 1), from it (2), into the place it is saved while a turn runs again (3), or back from there (4)\nfn __hw_region(k: i64, s: i64, dir: i64)\n    hr: ptr = addr __hw_reg\n    k2: i64 = mul k, 2\n    ki: index = conv k2\n    ow: i64 = load hr, ki, 8\n    k3: index = add ki, 1\n    lw: i64 = load hr, k3, 8\n    o: index = conv ow\n    len: index = conv lw\n    ab: ptr = addr __arena\n    base: ptr = load ab\n    rp: ptr(u8) = ptradd base, o\n    r: u8[] = pack rp, len, 1\n    sb: i64 = mul s, {r}\n    so: index = conv sb\n    smp: ptr = addr __hw_sm\n    sq: ptr(u8) = ptradd smp, so\n    sv: u8[] = pack sq, len, 1\n    svp: ptr = addr __hw_save\n    svq: ptr(u8) = cast svp\n    sav: u8[] = pack svq, len, 1\n    d1: u1 = cmp.eq dir, 1\n    if d1\n        copy sv, r\n    d2: u1 = cmp.eq dir, 2\n    if d2\n        copy r, sv\n    d3: u1 = cmp.eq dir, 3\n    if d3\n        copy sav, r\n    d4: u1 = cmp.eq dir, 4\n    if d4\n        copy r, sav\n    ret\n", r = REGION));
    // sending a kept turn
    t.push_str(&format!("\n; (hop fifty-nine) kept turn s of person k, whose push is for t, has been taken: how much its device wrote is noted, and it is sent to the home over k's wire, held while the wire is gone\nfn __hw_send(k: i64, s: i64, t: i64)\n    c: ptr = context()\n    x: __ctx = load c\n    on: index = get x, __out_n\n    onw: i64 = conv on\n    sk: ptr = addr __hw_sk\n    b0: i64 = mul s, 8\n    b2: i64 = add b0, 2\n    i2: index = conv b2\n    store onw, sk, i2, 8\n    now: i64 = get x, __clock\n    go: i64 = __hw_up(k, now)\n    hp: ptr = addr __hw\n    di: i64 = add k, 1\n    dii: index = conv di\n    d: i64 = load hp, dii, 8\n    at: i64 = add go, d\n    __hw_put(at, 1, k, now, s)\n    ret\n"));
    t.push_str(&format!("\n; (hop fifty-nine) an arrival put on the wire: when, what, whose, the push's own time, and its turn or the home's values\nfn __hw_put(at: i64, kind: i64, k: i64, t: i64, s: i64)\n    ev: ptr = addr __hw_ev\n    hp: ptr = addr __hw\n    hi: i64 = load hp, 320\n    e: i64 = loop(i: i64 = 0)\n        room: u1 = cmp.lt i, {n}\n        check room\n        b: i64 = mul i, 8\n        b5: i64 = add b, 5\n        bi: index = conv b5\n        live: i64 = load ev, bi, 8\n        free: u1 = cmp.eq live, 0\n        if free\n            break i\n        i2: i64 = add i, 1\n        continue i2\n    b0: i64 = mul e, 8\n    i0: index = conv b0\n    store at, ev, i0, 8\n    i1: index = add i0, 1\n    store kind, ev, i1, 8\n    i2b: index = add i0, 2\n    store k, ev, i2b, 8\n    i3: index = add i0, 3\n    store t, ev, i3, 8\n    i4: index = add i0, 4\n    store s, ev, i4, 8\n    i5: index = add i0, 5\n    store 1: i64, ev, i5, 8\n    e1: i64 = add e, 1\n    hi2: i64 = max(hi, e1)\n    store hi2, hp, 320\n    ret\n", n = EVENTS));
    // the earliest arrival: by when, a push at the home before the home's values at a copy, then the push's own time, then whose
    t.push_str("\n; (hop fifty-nine) the earliest arrival on the wires, or none: by when it arrives; of two at once a push at the home before the home's values at a copy, then by the push's own time, then by the order the case names the people (fm3 places.md, the amendment of point 3: by time, then by place)\nfn __hw_first() -> i64\n    ev: ptr = addr __hw_ev\n    hp: ptr = addr __hw\n    hi: i64 = load hp, 320\n    r: i64, ra: i64, rk: i64, rt: i64, rp: i64 = loop(i: i64 = 0, best: i64 = -1, ba: i64 = 4611686018427387904, bk: i64 = 9, bt: i64 = 0, bp: i64 = 9)\n        id: u1 = cmp.ge i, hi\n        if id\n            break best, ba, bk, bt, bp\n        b: i64 = mul i, 8\n        bi: index = conv b\n        b5: index = add bi, 5\n        live: i64 = load ev, b5, 8\n        a: i64 = load ev, bi, 8\n        b1: index = add bi, 1\n        kd: i64 = load ev, b1, 8\n        b2: index = add bi, 2\n        pp: i64 = load ev, b2, 8\n        b3: index = add bi, 3\n        st: i64 = load ev, b3, 8\n        on: u1 = cmp.ne live, 0\n        lt: u1 = cmp.lt a, ba\n        eq: u1 = cmp.eq a, ba\n        kl: u1 = cmp.lt kd, bk\n        ke: u1 = cmp.eq kd, bk\n        tl: u1 = cmp.lt st, bt\n        te: u1 = cmp.eq st, bt\n        pl: u1 = cmp.lt pp, bp\n        tp: u1 = and te, pl\n        tt: u1 = or tl, tp\n        kt: u1 = and ke, tt\n        kk: u1 = or kl, kt\n        ek: u1 = and eq, kk\n        better: u1 = or lt, ek\n        take: u1 = and on, better\n        i2: i64 = add i, 1\n        if take\n            continue i2, i, a, kd, st, pp\n        continue i2, best, ba, bk, bt, bp\n    ret r\n");
    t.push_str("\n; (hop fifty-nine) when the earliest arrival is due, if no later than t\nfn __hw_soon(t: i64) -> i64\n    e: i64 = __hw_first()\n    never: i64 = const 4611686018427387904\n    none: u1 = cmp.lt e, 0\n    if none\n        ret never\n    ev: ptr = addr __hw_ev\n    b: i64 = mul e, 8\n    bi: index = conv b\n    a: i64 = load ev, bi, 8\n    late: u1 = cmp.gt a, t\n    if late\n        ret never\n    ret a\n");
    // running a kept turn again against given values, its text written nowhere
    let mut calls = String::new();
    for (id, (_, g)) in twins.iter().enumerate() {
        calls.push_str(&format!("    c{id}: u1 = cmp.eq kind, {k}\n    if c{id}\n        {g}()\n", id = id, k = id + 2, g = g));
    }
    t.push_str(&format!("\n; (hop fifty-nine) kept turn s run again against the values at gp and lp, which it leaves as the turn leaves them: in a context of its own, the kept one with a capture nobody reads, and its device's region of the arena as it was when the turn began, put back after. Gives how much it wrote\nfn __hw_again(s: i64, gp: ptr, lp: ptr) -> i64\n    was: ptr = context()\n    sk: ptr = addr __hw_sk\n    b0: i64 = mul s, 8\n    i0: index = conv b0\n    k: i64 = load sk, i0, 8\n    i1: index = add i0, 1\n    kind: i64 = load sk, i1, 8\n    __hw_region(k, s, 3)\n    __hw_region(k, s, 2)\n    scm: ptr(array(__ctx, {n})) = addr __hw_sc\n    si: index = conv s\n    se: ptr(__ctx) = index scm, si\n    sp: ptr = cast se\n    x: __ctx = load sp\n    cp: ptr = addr __hw_cap\n    mp: ptr = addr __hw_capm\n    x1: __ctx = set x, __out_p, cp\n    x2: __ctx = set x1, __out_n, 0\n    x3: __ctx = set x2, __out_m, mp\n    xp: ptr = addr __hw_x\n    store x3, xp\n    context_set(xp)\n    __hw_in(gp, lp)\n    key: u1 = cmp.eq kind, 1\n    if key\n        __in_turn()\n{calls}    __hw_out(gp, lp)\n    y: __ctx = load xp\n    wn: index = get y, __out_n\n    w: i64 = conv wn\n    context_set(was)\n    __hw_region(k, s, 4)\n    ret w\n", n = SNAPS, calls = calls));
    // the home's values kept to be sent, with how much of each person's each has had
    t.push_str(&format!("\n; (hop fifty-nine) the home's values as they are now, kept to cross to the copies, and how many of each person's pushes they have had: which place they are kept in\nfn __hw_keep() -> i64\n    hp: ptr = addr __hw\n    n: i64 = load hp, 336\n    n2: i64 = add n, 1\n    store n2, hp, 336\n    v: i64 = rem n, {p}\n    vi: index = conv v\n    hg: ptr, hl: ptr = __hw_homes()\n{g}{l}    pa: ptr = addr __hw_pa\n    v4: i64 = mul v, 4\n    loop(j: i64 = 0) bound 4\n        jd: u1 = cmp.ge j, 4\n        if jd\n            break\n        aj: i64 = add j, 48\n        ai: index = conv aj\n        a: i64 = load hp, ai, 8\n        pj: i64 = add v4, j\n        pji: index = conv pj\n        store a, pa, pji, 8\n        hm: ptr = addr __hw_m\n        pm: ptr = addr __hw_pm\n        v8: i64 = mul v, 8\n        j2x: i64 = mul j, 2\n        pm0: i64 = add v8, j2x\n        pm0i: index = conv pm0\n        pm1i: index = add pm0i, 1\n        mji: index = conv j\n        md: i64 = load hm, mji, 8\n        mg8: i64 = add j, 8\n        mgi: index = conv mg8\n        mg: i64 = load hm, mgi, 8\n        store md, pm, pm0i, 8\n        store mg, pm, pm1i, 8\n        j2: i64 = add j, 1\n        continue j2\n    ret v\n", p = PAYLOADS, g = if gp { format!("    gvm: ptr(array(__group, {p})) = addr __hw_gv\n    gve: ptr(__group) = index gvm, vi\n    gvp: ptr = cast gve\n    gs: __group = load hg\n    store gs, gvp\n", p = PAYLOADS) } else { String::new() }, l = if lp { format!("    lvm: ptr(array(__global, {p})) = addr __hw_lv\n    lve: ptr(__global) = index lvm, vi\n    lvp: ptr = cast lve\n    ls: __global = load hl\n    store ls, lvp\n", p = PAYLOADS) } else { String::new() }));
    // taking the earliest arrival
    t.push_str(&format!("\n; (hop fifty-nine) the earliest arrival taken. A push at the home: its turn run again against the home's values, which it leaves as the line says (the line is the merge), the record noting when it arrived and whether what its device showed is what the home's order gives; and the home's values sent to every copy. The home's values at a copy: the copy takes them, and runs again on top each push of its own the home had not had\nfn __hw_take()\n    e: i64 = __hw_first()\n    ev: ptr = addr __hw_ev\n    b: i64 = mul e, 8\n    bi: index = conv b\n    at: i64 = load ev, bi, 8\n    b1: index = add bi, 1\n    kind: i64 = load ev, b1, 8\n    b2: index = add bi, 2\n    k: i64 = load ev, b2, 8\n    b3: index = add bi, 3\n    st: i64 = load ev, b3, 8\n    b4: index = add bi, 4\n    s: i64 = load ev, b4, 8\n    b5: index = add bi, 5\n    store 0: i64, ev, b5, 8\n    hp: ptr = addr __hw\n    sp: ptr = addr __share\n    m: i64 = load sp\n    k0: i64 = load sp, 8\n    home: u1 = cmp.eq kind, 1\n    if home\n        same: u1 = __hw_order(s)\n        ak: i64 = add k, 48\n        aki: index = conv ak\n        a0: i64 = load hp, aki, 8\n        a1: i64 = add a0, 1\n        store a1, hp, aki, 8\n        di: i64 = add k, 1\n        dii: index = conv di\n        d: i64 = load hp, dii, 8\n        sw: i64 = conv same\n        __hw_note(1, k, st, at, d, sw)\n        up: i64 = sub at, st\n        __hw_measure(k, up)\n        v: i64 = __hw_keep()\n        loop(j: i64 = 0) bound 4\n            jm: u1 = cmp.lt j, m\n            if jm\n                go: i64 = __hw_up(j, at)\n                dj: i64 = add j, 1\n                dji: index = conv dj\n                dd: i64 = load hp, dji, 8\n                aj: i64 = add go, dd\n                __hw_put(aj, 2, j, at, v)\n                j2: i64 = add j, 1\n                continue j2\n            break\n        ret\n    k32: i32 = conv k\n    __zero_context(k32)\n    vi: index = conv s\n{gin}{lin}    __hw_in(gvp, lvp)\n    pa: ptr = addr __hw_pa\n    s4: i64 = mul s, 4\n    pk4: i64 = add s4, k\n    pki: index = conv pk4\n    had: i64 = load pa, pki, 8\n    mk: i64 = add k, 44\n    mki: index = conv mk\n    made: i64 = load hp, mki, 8\n    hk: ptr = addr __hw_pk\n    sk2: ptr = addr __hw_sk\n    {tg}\n    {tl}\n    loop(i: i64 = had)\n        id: u1 = cmp.ge i, made\n        if id\n            break\n        km: i64 = mul k, 64\n        im: i64 = rem i, 64\n        pi: i64 = add km, im\n        pii: index = conv pi\n        ps: i64 = load hk, pii, 8\n        __hw_out(tgp, tlp)\n        r: i64 = __hw_again(ps, tgp, tlp)\n        __hw_in(tgp, tlp)\n        i2: i64 = add i, 1\n        continue i2\n    loop(q: i64 = 0) bound {n}\n        qd: u1 = cmp.ge q, {n}\n        if qd\n            break\n        q8: i64 = mul q, 8\n        q0i: index = conv q8\n        qk: i64 = load sk2, q0i, 8\n        q5: index = add q0i, 5\n        qn: i64 = load sk2, q5, 8\n        q7: index = add q0i, 7\n        mine: u1 = cmp.eq qk, k\n        old: u1 = cmp.lt qn, had\n        done: u1 = and mine, old\n        qw: i64 = load sk2, q7, 8\n        qwait: u1 = cmp.ne qw, 0\n        fresh: u1 = and done, qwait\n        if fresh\n            q3: index = add q0i, 3\n            qst: i64 = load sk2, q3, 8\n            rtt: i64 = sub at, qst\n            kx: i64 = add k, 4\n            __hw_measure(kx, rtt)\n        if done\n            store 0: i64, sk2, q7, 8\n        q2: i64 = add q, 1\n        continue q2\n    hm2: ptr = addr __hw_m\n    pm2: ptr = addr __hw_pm\n    s8m: i64 = mul s, 8\n    loop(w: i64 = 0) bound 8\n        wd: u1 = cmp.ge w, 8\n        if wd\n            break\n        wsrc: i64 = add s8m, w\n        wsi: index = conv wsrc\n        wv: i64 = load pm2, wsi, 8\n        k8: i64 = mul k, 8\n        k8b: i64 = add k8, 32\n        wdst: i64 = add k8b, w\n        wdi: index = conv wdst\n        store wv, hm2, wdi, 8\n        w2: i64 = add w, 1\n        continue w2\n    __hw_note(2, k, st, at, had, made)\n    k0w: i32 = conv k0\n    __zero_context(k0w)\n    ret\n", n = SNAPS, gin = if gp { format!("    gvm: ptr(array(__group, {p})) = addr __hw_gv\n    gve: ptr(__group) = index gvm, vi\n    gvp: ptr = cast gve\n", p = PAYLOADS) } else { "    gvp: ptr = const 0\n".to_string() }, lin = if lp { format!("    lvm: ptr(array(__global, {p})) = addr __hw_lv\n    lve: ptr(__global) = index lvm, vi\n    lvp: ptr = cast lve\n", p = PAYLOADS) } else { "    lvp: ptr = const 0\n".to_string() }, tg = if gp { "tgp: ptr = addr __hw_tg" } else { "tgp: ptr = const 0" }, tl = if lp { "tlp: ptr = addr __hw_tl" } else { "tlp: ptr = const 0" }));
    // the home's order: by the push's own time, then by place
    t.push_str(&format!("\n; (hop fifty-nine) kept turn s, a push that has reached the home, put in the home's order: by its own time, then by the order the case names the people (fm3 places.md, the amendment of point 3). Where it sorts after every push the home has had it is run again against the home's values; where it sorts before some, the home's values go back to what they were before the first the home still keeps and every one is run again in that order. The home keeps the last {keep}, and a push that sorts before all of those is put after the first. Gives whether what its device showed is what the home's order gives\nfn __hw_order(s: i64) -> u1\n    hp: ptr = addr __hw\n    n: i64 = load hp, 424\n    sk: ptr = addr __hw_sk\n    ho: ptr = addr __hw_ho\n    b0: i64 = mul s, 8\n    i0: index = conv b0\n    k: i64 = load sk, i0, 8\n    i3: index = add i0, 3\n    st: i64 = load sk, i3, 8\n    hg: ptr, hl: ptr = __hw_homes()\n    empty: u1 = cmp.eq n, 0\n    if empty\n        __hw_base(1)\n    pos: i64 = loop(i: i64 = 0)\n        id: u1 = cmp.ge i, n\n        if id\n            break n\n        ii: index = conv i\n        si: i64 = load ho, ii, 8\n        sb: i64 = mul si, 8\n        sbi: index = conv sb\n        ki: i64 = load sk, sbi, 8\n        sb3: index = add sbi, 3\n        ti: i64 = load sk, sb3, 8\n        later: u1 = cmp.gt ti, st\n        same: u1 = cmp.eq ti, st\n        kl: u1 = cmp.gt ki, k\n        sk2: u1 = and same, kl\n        after: u1 = or later, sk2\n        if after\n            break i\n        i2: i64 = add i, 1\n        continue i2\n    loop(j: i64 = n)\n        jd: u1 = cmp.le j, pos\n        if jd\n            break\n        j1: i64 = sub j, 1\n        j1i: index = conv j1\n        v: i64 = load ho, j1i, 8\n        ji: index = conv j\n        store v, ho, ji, 8\n        continue j1\n    pi: index = conv pos\n    store s, ho, pi, 8\n    n2: i64 = add n, 1\n    store n2, hp, 424\n    last: u1 = cmp.eq pos, n\n    r: u1 = if last\n        w: i64 = __hw_again(s, hg, hl)\n        sm: u1 = __hw_same(s, w)\n        yield sm\n    else\n        __hw_base(2)\n        r2: u1 = loop(q: i64 = 0, got: u1 = 1)\n            qd: u1 = cmp.ge q, n2\n            if qd\n                break got\n            qi: index = conv q\n            qs: i64 = load ho, qi, 8\n            wq: i64 = __hw_again(qs, hg, hl)\n            mine: u1 = cmp.eq qs, s\n            sq: u1 = __hw_same(qs, wq)\n            other: u1 = xor mine, 1\n            nsq: u1 = xor sq, 1\n            moved: u1 = and other, nsq\n            if moved\n                qb: i64 = mul qs, 8\n                qbi: index = conv qb\n                qk: i64 = load sk, qbi, 8\n                qb3: index = add qbi, 3\n                qt: i64 = load sk, qb3, 8\n                __hw_note(3, qk, qt, 0, 0, 0)\n            g2: u1 = if mine\n                yield sq\n            else\n                yield got\n            q2: i64 = add q, 1\n            continue q2, g2\n        yield r2\n    loop(f: i64 = 0)\n        nn: i64 = load hp, 424\n        over: u1 = cmp.gt nn, {keep}\n        if over\n        else\n            break\n        f0: i64 = load ho\n        bgp: ptr, blp: ptr = __hw_bases()\n        wf: i64 = __hw_again(f0, bgp, blp)\n        fb: i64 = mul f0, 8\n        f6: i64 = add fb, 6\n        f6i: index = conv f6\n        store 0: i64, sk, f6i, 8\n        loop(m: i64 = 1)\n            md: u1 = cmp.ge m, nn\n            if md\n                break\n            mi: index = conv m\n            mv: i64 = load ho, mi, 8\n            m1: i64 = sub m, 1\n            m1i: index = conv m1\n            store mv, ho, m1i, 8\n            m2: i64 = add m, 1\n            continue m2\n        nn2: i64 = sub nn, 1\n        store nn2, hp, 424\n        f2: i64 = add f, 1\n        continue f2\n    ret r\n", keep = KEEP));
    t.push_str(&format!("\n; (hop fifty-nine) where the home's values were before the first push it keeps\nfn __hw_bases() -> (ptr, ptr)\n{g}{l}    ret bg, bl\n\n; (hop fifty-nine) the home's values kept as those before the first push it keeps (1), or put back to them (2)\nfn __hw_base(dir: i64)\n    hg: ptr, hl: ptr = __hw_homes()\n    bg: ptr, bl: ptr = __hw_bases()\n    keep: u1 = cmp.eq dir, 1\n{gk}{lk}    ret\n", g = if gp { "    bg: ptr = addr __hw_bg\n" } else { "    bg: ptr = const 0\n" }, l = if lp { "    bl: ptr = addr __hw_bl\n" } else { "    bl: ptr = const 0\n" }, gk = if gp { "    if keep\n        g1: __group = load hg\n        store g1, bg\n    else\n        g2: __group = load bg\n        store g2, hg\n" } else { "" }, lk = if lp { "    if keep\n        l1: __global = load hl\n        store l1, bl\n    else\n        l2: __global = load bl\n        store l2, hl\n" } else { "" }));
    // a wire measured: the last, the least, the gaps and the time gone in them
    t.push_str("\n; (hop fifty-nine) a wire measured where it ends: w 0 to 3 a person's wire as the home sees it, one way, 4 to 7 as the person's device sees it, a round trip; the last measure, the least, how many were more than twice the least (a gap: the wire gone or slow), and the time more than the least in them\nfn __hw_measure(w: i64, d: i64)\n    hm: ptr = addr __hw_m\n    four: u1 = cmp.ge w, 4\n    base: i64 = if four\n        x: i64 = sub w, 4\n        y: i64 = add x, 16\n        yield y\n    else\n        yield w\n    bi: index = conv base\n    store d, hm, bi, 8\n    li: index = add bi, 4\n    l: i64 = load hm, li, 8\n    none: u1 = cmp.eq l, 0\n    less: u1 = cmp.lt d, l\n    nl: u1 = or none, less\n    l2: i64 = if nl\n        yield d\n    else\n        yield l\n    store l2, hm, li, 8\n    l22: i64 = mul l2, 2\n    gap: u1 = cmp.gt d, l22\n    if gap\n        gi: index = add bi, 8\n        g: i64 = load hm, gi, 8\n        g2: i64 = add g, 1\n        store g2, hm, gi, 8\n        ti: index = add bi, 12\n        tg: i64 = load hm, ti, 8\n        ov: i64 = sub d, l2\n        tg2: i64 = add tg, ov\n        store tg2, hm, ti, 8\n    ret\n");
    // what the device showed against what the home's order gives
    t.push_str("\n; (hop fifty-nine) did kept turn s write, run again at the home, the w characters its device wrote when it was taken\nfn __hw_same(s: i64, w: i64) -> u1\n    sk: ptr = addr __hw_sk\n    b0: i64 = mul s, 8\n    i0: index = conv b0\n    k: i64 = load sk, i0, 8\n    i4: index = add i0, 4\n    o0: i64 = load sk, i4, 8\n    i2: index = add i0, 2\n    o1: i64 = load sk, i2, 8\n    n: i64 = sub o1, o0\n    eqn: u1 = cmp.eq n, w\n    if eqn\n    else\n        ret 0\n    cm: ptr(array(__ctx, 4)) = addr __ctx_mem\n    ki: index = conv k\n    ce: ptr(__ctx) = index cm, ki\n    cq: ptr = cast ce\n    c: __ctx = load cq\n    dp: ptr = get c, __out_p\n    hc: ptr = addr __hw_cap\n    r: u1 = loop(i: i64 = 0)\n        id: u1 = cmp.ge i, n\n        if id\n            break 1\n        j: i64 = add o0, i\n        ji: index = conv j\n        a: u8 = load dp, ji, 1\n        ii: index = conv i\n        h: u8 = load hc, ii, 1\n        ne: u1 = cmp.ne a, h\n        if ne\n            break 0\n        ix: i64 = add i, 1\n        continue ix\n    ret r\n");
    // at the end, each shared stream at the home and at each copy
    let mut end = String::from("\n; (hop fifty-nine) at the end of a case, the record says each shared stream kept as a word at its home and at each copy: 4, the stream, the place (-1 the home), the value\nfn __hw_end()\n    sp: ptr = addr __share\n    m: i64 = load sp\n    cm: ptr(array(__ctx, 4)) = addr __ctx_mem\n    hg: ptr, hl: ptr = __hw_homes()\n");
    let mut fi = 0;
    for (kind, fs) in [("group", group), ("global", global)] {
        for (f, ty) in fs {
            let src = if kind == "group" { "hg" } else { "hl" };
            end.push_str(&format!("    hs{fi}: __{kind} = load {src}\n    hv{fi}: {ty} = get hs{fi}, {f}\n    hw{fi}: i64 = conv hv{fi}\n    __hw_note(4, {fi}, -1, hw{fi}, 0, 0)\n    loop(j{fi}: i64 = 0) bound 4\n        jm{fi}: u1 = cmp.lt j{fi}, m\n        if jm{fi}\n            ji{fi}: index = conv j{fi}\n            ce{fi}: ptr(__ctx) = index cm, ji{fi}\n            cq{fi}: ptr = cast ce{fi}\n            cx{fi}: __ctx = load cq{fi}\n            cv{fi}: {ty} = get cx{fi}, {f}\n            cw{fi}: i64 = conv cv{fi}\n            __hw_note(4, {fi}, j{fi}, cw{fi}, 0, 0)\n            j2{fi}: i64 = add j{fi}, 1\n            continue j2{fi}\n        break\n", fi = fi, kind = kind, src = src, ty = ty, f = f));
            fi += 1;
        }
    }
    // ... and what each device has measured of its own wire and been told of every other: 5, the device, its last round trip, the least, its gaps, the time in them; 6, the device, a person, that wire's last delay and gaps as the home told it
    end.push_str("    hm: ptr = addr __hw_m\n    loop(dv: i64 = 0) bound 4\n        dvm: u1 = cmp.lt dv, m\n        if dvm\n        else\n            break\n        o0: i64 = add dv, 16\n        o0i: index = conv o0\n        r0: i64 = load hm, o0i, 8\n        o1i: index = add o0i, 4\n        r1: i64 = load hm, o1i, 8\n        o2i: index = add o0i, 8\n        r2: i64 = load hm, o2i, 8\n        o3i: index = add o0i, 12\n        r3: i64 = load hm, o3i, 8\n        __hw_note(5, dv, r0, r1, r2, r3)\n        loop(ov: i64 = 0) bound 4\n            ovm: u1 = cmp.lt ov, m\n            if ovm\n            else\n                break\n            d8: i64 = mul dv, 8\n            d8b: i64 = add d8, 32\n            o2x: i64 = mul ov, 2\n            tx: i64 = add d8b, o2x\n            txi: index = conv tx\n            td: i64 = load hm, txi, 8\n            txj: index = add txi, 1\n            tg: i64 = load hm, txj, 8\n            __hw_note(6, dv, ov, td, tg, 0)\n            ov2: i64 = add ov, 1\n            continue ov2\n        dv2: i64 = add dv, 1\n        continue dv2\n    ret\n");
    t.push_str(&end);
    t
}

/// the names of the shared streams kept as a word, in the record's order
pub fn shared_names(ir: &str) -> Vec<String> {
    let (lines, _) = parse(ir);
    fields_of(&lines, "__group").into_iter().chain(fields_of(&lines, "__global")).map(|(n, _)| n).collect()
}
