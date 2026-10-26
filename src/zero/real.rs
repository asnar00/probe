//! The real wire, step one (fm3 wire.md; hop sixty): the two places of a
//! placed build in two processes on this machine, each with memory of its
//! own and its own clock, and exactly the rows that cross the simulated
//! wire carried between them as bytes over a loopback socket.
//!
//! The runner compiles the store's build for a wire once, with the queue's
//! entries `places::Placed::real_text` adds, and forks a child for the
//! server, as it forks a child for every call: the server binds
//! `127.0.0.1:0` and says its port on its pipe; then a child for the
//! client, which connects. Each child keeps the one module and uses its
//! own context only. At each the harness here is the world loop
//! (`__world`'s at one place): what arrived is handed to the module's
//! queue, the place delivers and takes the turns that are due by the four
//! conditions on the real clock, and what it put on is written to the
//! socket. The case ends when both places have nothing left to do and each
//! has had everything the other sent. The client says its text, marks and
//! record on its pipe as a call does; the server its record; the runner
//! joins the two records and compares the case's line by the pieces' own
//! times, as over the simulated wire.

use super::places::Placed;
use super::run::{self, Planned};
use super::{lower, store};
use crate::emit::jit::JitCode;
use crate::suite::{self, Report};
use crate::{emit, ssa};
use std::io::{BufRead, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::Path;
use std::time::{Duration, Instant};

/// the never-time of the world's text
const NEVER: i64 = 1 << 62;
/// the harness's own rows, which the module never sees: a ping and its
/// answer (fm3 wire.md, the kinds 13 and 14), and a place's word that it
/// has nothing left, with how many rows it has sent and had
const PING: i64 = 13;
const PONG: i64 = 14;
const SAID_END: i64 = 30;
/// ... and the client's word that its case begins: what arrived before it
/// was there when the case began, the two places having been joined
/// before it, and the server's clock starts when it arrives
const BEGINS: i64 = 31;
/// ... and the end of what a place put on in one pass of its world, the
/// rows before it handed to the module at the other place together (hop
/// sixty-three): a turn's rows arrive at one moment over the simulated
/// wire, and are delivered before the place arrived at says anything
const BURST: i64 = 32;
/// how many pings the client sends before its case begins
const PINGS: i64 = 16;
/// a row's head on the socket: its eight words and its payload's length
const HEAD: usize = 9 * 8;
/// the kinds of the module's rows that are items of the wire, as its
/// record has them (the rest of its lines are notes)
const ITEMS: [i64; 11] = [1, 2, 3, 4, 5, 10, 11, 12, 13, 14, 17];
/// the longest a case may take on a real wire before its two processes
/// are stopped, in seconds
const LIMIT: u32 = 120;

/// A real wire as the command line says it: `--wire real`, and a set
/// delay, `--wire real, 100 ms`, which the receiver holds each row for
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Real {
    pub delay: i64,
}

impl Real {
    /// as a person reads it: `real`, `real, 100 ms`
    pub fn said(&self) -> String {
        if self.delay == 0 { "real".to_string() } else { format!("real, {}", run::spell_late(self.delay)) }
    }
}

/// What one place of a real run says when its case has ended: at the
/// client what the call gave, the text, its marks and when each character
/// was shown; at both the wire's record, when each row from the other
/// arrived by its clock, what it put on the socket and had, the pings'
/// round trips and how long the case took; at the server how many turns
/// it took ahead and threw away
#[derive(Clone, Debug, Default)]
pub struct Side {
    pub values: Vec<i64>,
    pub text: String,
    pub own: Vec<i64>,
    pub shown: Vec<i64>,
    pub log: Vec<i64>,
    pub arrived: Vec<i64>,
    pub rows_out: i64,
    pub bytes_out: i64,
    pub rows_in: i64,
    pub bytes_in: i64,
    pub pings: Vec<i64>,
    pub took: i64,
    pub thrown: i64,
}

impl Side {
    fn said(&self) -> String {
        let n = |v: &[i64]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>().join(",");
        let bytes: Vec<i64> = self.text.bytes().map(|b| b as i64).collect();
        format!("values={}\ntext={}\nown={}\nshown={}\nlog={}\narrived={}\ncounts={},{},{},{},{},{}\npings={}\n", n(&self.values), n(&bytes), n(&self.own), n(&self.shown), n(&self.log), n(&self.arrived), self.rows_out, self.bytes_out, self.rows_in, self.bytes_in, self.took, self.thrown, n(&self.pings))
    }

    fn read(text: &str) -> Result<Side, String> {
        let mut s = Side::default();
        for line in text.lines() {
            let Some((k, v)) = line.split_once('=') else { continue };
            let nums: Vec<i64> = if v.is_empty() { Vec::new() } else { v.split(',').map(|x| x.parse::<i64>().map_err(|e| format!("{}: {}", k, e))).collect::<Result<_, _>>()? };
            match k {
                "values" => s.values = nums,
                "text" => s.text = String::from_utf8_lossy(&nums.iter().map(|b| *b as u8).collect::<Vec<u8>>()).to_string(),
                "own" => s.own = nums,
                "shown" => s.shown = nums,
                "log" => s.log = nums,
                "arrived" => s.arrived = nums,
                "pings" => s.pings = nums,
                "counts" if nums.len() == 6 => {
                    (s.rows_out, s.bytes_out, s.rows_in, s.bytes_in, s.took, s.thrown) = (nums[0], nums[1], nums[2], nums[3], nums[4], nums[5]);
                }
                _ => {}
            }
        }
        Ok(s)
    }
}

unsafe extern "C" {
    fn fork() -> i32;
    fn waitpid(pid: i32, status: *mut i32, options: i32) -> i32;
    fn _exit(code: i32) -> !;
    fn pipe(fds: *mut i32) -> i32;
    fn alarm(seconds: u32) -> u32;
    fn select(nfds: i32, read: *mut FdSet, write: *mut FdSet, error: *mut FdSet, timeout: *mut TimeVal) -> i32;
}

/// `fd_set` and `struct timeval` as the system lays them out
#[repr(C)]
struct FdSet([i32; 32]);
#[repr(C)]
struct TimeVal {
    sec: i64,
    usec: i32,
}

/// One end of the socket: the bytes read and not yet a whole row, what
/// went each way, and when each of the module's rows arrived
struct Link {
    sock: TcpStream,
    buf: Vec<u8>,
    eof: bool,
    rows_out: i64,
    bytes_out: i64,
    rows_in: i64,
    bytes_in: i64,
    arrived: Vec<i64>,
    /// the other place's last word that it has nothing left
    heard: Option<(i64, i64)>,
    /// the pings answered
    pongs: Vec<i64>,
    /// the module's rows arrived since the other's last end of a pass,
    /// not yet handed over
    held: Vec<([i64; 8], Vec<i64>)>,
    /// rows written since this place's last end of a pass
    unmarked: i64,
}

impl Link {
    fn new(sock: TcpStream) -> Result<Link, String> {
        sock.set_nodelay(true).map_err(|e| e.to_string())?;
        sock.set_nonblocking(true).map_err(|e| e.to_string())?;
        Ok(Link { sock, buf: Vec::new(), eof: false, rows_out: 0, bytes_out: 0, rows_in: 0, bytes_in: 0, arrived: Vec::new(), heard: None, pongs: Vec::new(), held: Vec::new(), unmarked: 0 })
    }

    /// a row on the socket (fm3 wire.md, "what a row is on the wire"): its
    /// eight words little-endian, its payload's length in bytes, its payload
    fn send(&mut self, w: &[i64; 8], payload: &[i64], item: bool) -> Result<(), String> {
        let mut bytes = Vec::with_capacity(HEAD + 8 * payload.len());
        for v in w.iter().chain(std::iter::once(&(8 * payload.len() as i64))).chain(payload) {
            bytes.extend_from_slice(&v.to_le_bytes());
        }
        if item {
            self.rows_out += 1;
            self.bytes_out += bytes.len() as i64;
        }
        let mut at = 0;
        while at < bytes.len() {
            match self.sock.write(&bytes[at..]) {
                Ok(0) => return Err("the wire closed while a row was being written".into()),
                Ok(n) => at += n,
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => std::thread::sleep(Duration::from_micros(50)),
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
                Err(e) => return Err(format!("the wire: {}", e)),
            }
        }
        Ok(())
    }

    /// what has arrived, without waiting
    fn fill(&mut self) -> Result<(), String> {
        let mut tmp = [0u8; 16384];
        loop {
            match self.sock.read(&mut tmp) {
                Ok(0) => {
                    self.eof = true;
                    return Ok(());
                }
                Ok(n) => self.buf.extend_from_slice(&tmp[..n]),
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => return Ok(()),
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
                Err(e) => return Err(format!("the wire: {}", e)),
            }
        }
    }

    /// a whole row has arrived and is not yet taken
    fn whole(&self) -> bool {
        self.buf.len() >= HEAD && self.buf.len() >= HEAD + i64::from_le_bytes(self.buf[64..72].try_into().unwrap()) as usize
    }

    /// wait until something arrives or `d` has passed
    fn wait(&mut self, d: Duration) -> Result<(), String> {
        if self.whole() || self.eof {
            return Ok(());
        }
        use std::os::unix::io::AsRawFd;
        let fd = self.sock.as_raw_fd();
        let mut set = FdSet([0; 32]);
        set.0[(fd / 32) as usize] |= 1 << (fd % 32);
        let mut tv = TimeVal { sec: d.as_secs() as i64, usec: d.subsec_micros() as i32 };
        unsafe { select(fd + 1, &mut set, std::ptr::null_mut(), std::ptr::null_mut(), &mut tv) };
        self.fill()
    }

    /// the next whole row: its eight words and its payload
    fn take(&mut self) -> Option<([i64; 8], Vec<i64>)> {
        if !self.whole() {
            return None;
        }
        let word = |b: &[u8], i: usize| i64::from_le_bytes(b[8 * i..8 * i + 8].try_into().unwrap());
        let mut w = [0i64; 8];
        for (i, x) in w.iter_mut().enumerate() {
            *x = word(&self.buf, i);
        }
        let len = word(&self.buf, 8) as usize;
        let payload: Vec<i64> = (0..len / 8).map(|k| word(&self.buf, 9 + k)).collect();
        self.buf.drain(..HEAD + len);
        Some((w, payload))
    }
}

/// a place's clock: its own, from when its case began, in microseconds;
/// nothing yet where it has not begun
fn clock(start: &Option<Instant>) -> i64 {
    start.map_or(0, |s| s.elapsed().as_micros() as i64)
}

fn call(jit: &JitCode, f: &str, args: &[i64]) -> Result<i64, String> {
    jit.call(f, args)
}

/// a row that has arrived: the harness's own answered or kept, a row of
/// the module's handed to its queue toward this place, when it arrives
/// being this place's clock now and the set delay; or, where this place's
/// case has not begun, the case's first moment, the two having been
/// joined before it (`__world`'s first lines: the places tell each other
/// where they stand, which takes no time)
fn arrive(jit: &JitCode, p: i64, link: &mut Link, start: &mut Option<Instant>, delay: i64) -> Result<(), String> {
    while let Some((w, payload)) = link.take() {
        match w[1] {
            PING => link.send(&[0, PONG, w[2], 0, 0, 0, 0, 0], &[], false)?,
            PONG => link.pongs.push(w[2]),
            SAID_END => link.heard = Some((w[2], w[3])),
            BEGINS => *start = Some(Instant::now()),
            BURST => {
                for (w, payload) in std::mem::take(&mut link.held) {
                    put(jit, p, link, start, delay, &w, &payload)?;
                }
            }
            _ => link.held.push((w, payload)),
        }
    }
    Ok(())
}

/// one of the module's rows handed to its queue toward this place, when
/// it arrives being this place's clock now and the set delay
fn put(jit: &JitCode, p: i64, link: &mut Link, start: &Option<Instant>, delay: i64, w: &[i64; 8], payload: &[i64]) -> Result<(), String> {
    let now = clock(start);
    let at = if start.is_some() { now + delay } else { 0 };
    link.rows_in += 1;
    link.bytes_in += (HEAD + 8 * payload.len()) as i64;
    link.arrived.push(now);
    call(jit, "__zero_context", &[p])?;
    for (j, v) in w.iter().enumerate().skip(1) {
        call(jit, "__wq_row_in", &[j as i64, *v])?;
    }
    for (k, v) in payload.iter().enumerate() {
        call(jit, "__wq_row_in", &[8 + k as i64, *v])?;
    }
    call(jit, "__wq_row_put", &[at])?;
    Ok(())
}

/// what this place has put on the wire, written to the socket
fn flush(jit: &JitCode, p: i64, link: &mut Link) -> Result<(), String> {
    call(jit, "__zero_context", &[p])?;
    let n = call(jit, "__wq_out", &[])?;
    for _ in 0..n {
        let mut w = [0i64; 8];
        for (j, x) in w.iter_mut().enumerate() {
            *x = call(jit, "__wq_row_out", &[j as i64])?;
        }
        let size = call(jit, "__wq_row_size", &[])?;
        let payload: Vec<i64> = (0..size).map(|k| call(jit, "__wq_row_out", &[8 + k])).collect::<Result<_, _>>()?;
        call(jit, "__wq_row_done", &[])?;
        link.send(&w, &payload, true)?;
        link.unmarked += 1;
    }
    Ok(())
}

/// the end of a pass of this place's world: what it wrote since the last
/// is one piece of the other's arrival (hop sixty-three)
fn mark(link: &mut Link) -> Result<(), String> {
    if link.unmarked > 0 {
        link.send(&[0, BURST, 0, 0, 0, 0, 0, 0], &[], false)?;
        link.unmarked = 0;
    }
    Ok(())
}

/// The world at one place (`__world`, each place in a process of its own):
/// until both places have nothing left, hand over what arrived, let the
/// place deliver what is due, take the turns it may and say where it
/// stands, and write what it put on to the socket; then wait for the next
/// thing due by its clock or for the socket. Gives how long its case took
fn world(jit: &JitCode, p: i64, link: &mut Link, start: &mut Option<Instant>, delay: i64) -> Result<i64, String> {
    let born = Instant::now();
    let mut said: Option<(i64, i64)> = None;
    loop {
        if born.elapsed() > Duration::from_secs(LIMIT as u64) {
            return Err(format!("the case did not end within {} s", LIMIT));
        }
        link.fill()?;
        arrive(jit, p, link, start, delay)?;
        loop {
            call(jit, "__wreal_now", &[clock(start)])?;
            // (before its case begins a place only says where it stands
            // and takes what it is told: no turn, as before `__world`'s
            // clock first moves)
            if call(jit, "__wdeliver", &[p])? & 1 != 0 || start.is_some() && call(jit, "__wturn", &[p])? & 1 != 0 {
                flush(jit, p, link)?;
                continue;
            }
            call(jit, "__zero_context", &[p])?;
            if call(jit, "__wsay", &[])? & 1 != 0 {
                flush(jit, p, link)?;
                continue;
            }
            break;
        }
        flush(jit, p, link)?;
        // (what this pass wrote is one piece of the other's arrival, as a
        // turn's rows arrive at one moment over the simulated wire: whether
        // the second of two rows had reached the other place before it said
        // where it stood was a race of the loopback, hop sixty-three)
        mark(link)?;
        let counts = (link.rows_out, link.rows_in);
        if call(jit, "__wreal_idle", &[p])? & 1 != 0 {
            if said != Some(counts) {
                link.send(&[0, SAID_END, counts.0, counts.1, 0, 0, 0, 0], &[], false)?;
                said = Some(counts);
            }
            if link.heard == Some((counts.1, counts.0)) {
                return Ok(clock(start));
            }
        }
        if link.eof && !link.whole() {
            return Err(format!("the {} closed the wire before the case ended", if p == 0 { "server" } else { "client" }));
        }
        let due = call(jit, "__wreal_due", &[p])?;
        let left = if due >= NEVER { 100_000 } else { (due - clock(start)).clamp(0, 100_000) };
        // (the system's timer wakes up to two milliseconds late: the last
        // of a wait is a poll of the socket)
        if left > 2_500 {
            link.wait(Duration::from_micros((left - 2_000) as u64))?;
        } else if left > 0 {
            link.wait(Duration::ZERO)?;
            std::thread::yield_now();
        }
    }
}

/// The record of a place, `__wlog`, read back as the runner reads it over
/// the simulated wire: six words an item
fn record(jit: &JitCode) -> Result<Vec<i64>, String> {
    call(jit, "__wire_mode", &[2])?;
    let n = call(jit, "__out_len", &[])?;
    (0..n).map(|i| call(jit, "__out_mark", &[i])).collect()
}

/// the server's child: listen, say the port, take the client's connection,
/// and run the server's part of the case
fn server_child(module: &ssa::Module, jit: &JitCode, cut: &[(String, Vec<i64>)], delay: i64, out: &mut std::fs::File) -> Result<Side, String> {
    let listener = TcpListener::bind("127.0.0.1:0").map_err(|e| e.to_string())?;
    let port = listener.local_addr().map_err(|e| e.to_string())?.port();
    out.write_all(format!("{}\n", port).as_bytes()).map_err(|e| e.to_string())?;
    out.flush().map_err(|e| e.to_string())?;
    listener.set_nonblocking(true).map_err(|e| e.to_string())?;
    let born = Instant::now();
    let sock = loop {
        match listener.accept() {
            Ok((s, _)) => break s,
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock && born.elapsed() < Duration::from_secs(10) => std::thread::sleep(Duration::from_millis(1)),
            Err(e) => return Err(format!("the server had no client: {}", e)),
        }
    };
    // (nothing is left listening: the one connection is all a case has)
    drop(listener);
    let mut link = Link::new(sock)?;
    call(jit, "__zero_context", &[0])?;
    call(jit, "__zero_reset", &[])?;
    for (f, args) in cut {
        call(jit, f, args)?;
    }
    call(jit, "__wreal_begin", &[1])?;
    let mut start = None;
    let took = world(jit, 1, &mut link, &mut start, delay)?;
    if module.func("__wreal_homes").is_some() {
        call(jit, "__wreal_homes", &[1])?;
    }
    let thrown = if module.func("__wreal_thrown").is_some() { call(jit, "__wreal_thrown", &[])? } else { 0 };
    let log = record(jit)?;
    Ok(Side { log, arrived: link.arrived.clone(), rows_out: link.rows_out, bytes_out: link.bytes_out, rows_in: link.rows_in, bytes_in: link.bytes_in, took, thrown, ..Default::default() })
}

/// the client's child: connect, make its context ready as a call's child
/// does, ping, and run the case's call and the client's part
#[allow(clippy::too_many_arguments)]
fn client_child(module: &ssa::Module, jit: &JitCode, port: u16, cut: &[(String, Vec<i64>)], setters: &[(String, Vec<i64>)], func: &str, args: &[i64], nrets: usize, delay: i64) -> Result<Side, String> {
    let born = Instant::now();
    let sock = loop {
        match TcpStream::connect(("127.0.0.1", port)) {
            Ok(s) => break s,
            Err(_) if born.elapsed() < Duration::from_secs(5) => std::thread::sleep(Duration::from_millis(2)),
            Err(e) => return Err(format!("the client could not reach the server: {}", e)),
        }
    };
    let mut link = Link::new(sock)?;
    call(jit, "__zero_context", &[0])?;
    call(jit, "__zero_reset", &[])?;
    for (f, a) in cut {
        call(jit, f, a)?;
    }
    call(jit, "__wreal_begin", &[0])?;
    for (f, a) in setters {
        call(jit, f, a)?;
    }
    if module.func("__zero_start").is_some() {
        call(jit, "__zero_start", &[])?;
    }
    // the case's call begun at its first moment, and the client says
    // where it stands before the clock moves
    let mut start: Option<Instant> = None;
    call(jit, "__wreal_now", &[0])?;
    call(jit, "__zero_context", &[0])?;
    call(jit, &format!("{}__begin", func), args)?;
    call(jit, "__zero_context", &[0])?;
    call(jit, "__wsay", &[])?;
    flush(jit, 0, &mut link)?;
    mark(&mut link)?;
    // the pings, while the server says where it stands: how long a row
    // takes there and back

    let mut pings = Vec::new();
    for k in 0..PINGS {
        let sent = Instant::now();
        link.send(&[0, PING, k, 0, 0, 0, 0, 0], &[], false)?;
        loop {
            if sent.elapsed() > Duration::from_secs(5) {
                return Err("the server did not answer a ping".into());
            }
            link.wait(Duration::from_millis(5))?;
            // (a row of the server's that comes first is the module's, kept as any that arrives before the case)
            arrive(jit, 0, &mut link, &mut start, delay)?;
            if link.pongs.contains(&k) {
                pings.push(sent.elapsed().as_nanos() as i64);
                break;
            }
            if link.eof {
                return Err("the server closed the wire before the case began".into());
            }
        }
    }
    // the case begins: the client's clock from its call, the server's
    // from when it hears so
    link.send(&[0, BEGINS, 0, 0, 0, 0, 0, 0], &[], false)?;
    start = Some(Instant::now());
    let took = world(jit, 0, &mut link, &mut start, delay)?;
    call(jit, "__zero_context", &[0])?;
    let end = format!("{}__end", func);
    let rets: Vec<ssa::Repr> = module.func(func).map(|f| f.rets.iter().map(|&t| f.repr(t)).collect()).unwrap_or_default();
    let fix = |i: usize, x: i64| match rets.get(i) {
        Some(r) if r.container() == 32 => crate::opt::norm(*r, x as u32 as i64),
        _ => x,
    };
    let values = match nrets {
        0 => {
            call(jit, &end, &[])?;
            Vec::new()
        }
        1 => vec![fix(0, call(jit, &end, &[])?)],
        2 => {
            let (a, b) = jit.call2(&end, &[])?;
            vec![fix(0, a), fix(1, b)]
        }
        n => return Err(format!("{} results not supported by the runner", n)),
    };
    if module.func("__wreal_homes").is_some() {
        call(jit, "__wreal_homes", &[0])?;
    }
    call(jit, "__wire_mode", &[0])?;
    let n = call(jit, "__out_len", &[])?;
    let bytes: Vec<u8> = (0..n).map(|i| call(jit, "__out_byte", &[i]).map(|b| b as u8)).collect::<Result<_, _>>()?;
    let own: Vec<i64> = (0..=n).map(|i| call(jit, "__out_mark", &[i])).collect::<Result<_, _>>()?;
    call(jit, "__wire_mode", &[1])?;
    let shown: Vec<i64> = (0..=n).map(|i| call(jit, "__out_mark", &[i])).collect::<Result<_, _>>()?;
    let log = record(jit)?;
    Ok(Side { values, text: String::from_utf8_lossy(&bytes).to_string(), own, shown, log, arrived: link.arrived.clone(), rows_out: link.rows_out, bytes_out: link.bytes_out, rows_in: link.rows_in, bytes_in: link.bytes_in, pings, took, thrown: 0 })
}

/// a child of the runner's: the pipe it writes to, and its pid
fn spawn<F: FnOnce(&mut std::fs::File) -> Result<Side, String>>(jit: &JitCode, f: F) -> Result<(i32, std::fs::File), String> {
    use std::os::unix::io::FromRawFd;
    let mut fds = [0i32; 2];
    if unsafe { pipe(fds.as_mut_ptr()) } != 0 {
        return Err("pipe failed".into());
    }
    let pid = unsafe { fork() };
    if pid < 0 {
        return Err("fork failed".into());
    }
    if pid == 0 {
        suite::child_traps(jit, fds[1]);
        drop(unsafe { std::fs::File::from_raw_fd(fds[0]) });
        unsafe { alarm(LIMIT + 10) };
        let mut w = unsafe { std::fs::File::from_raw_fd(fds[1]) };
        let text = match f(&mut w) {
            Ok(s) => format!("side\n{}", s.said()),
            Err(e) => format!("error: {}", e),
        };
        let _ = w.write_all(text.as_bytes());
        drop(w);
        unsafe { _exit(0) }
    }
    drop(unsafe { std::fs::File::from_raw_fd(fds[1]) });
    Ok((pid, unsafe { std::fs::File::from_raw_fd(fds[0]) }))
}

/// what a child said, once it has ended
fn ended(pid: i32, text: String, who: &str) -> Result<Side, String> {
    let mut status = 0i32;
    unsafe { waitpid(pid, &mut status, 0) };
    match status & 0x7f {
        0 => {}
        5 => return Err(format!("{} at the {}", suite::trap_said(""), who)),
        14 => return Err(format!("the {} was stopped after {} s", who, LIMIT + 10)),
        sig => return Err(format!("the {} died of signal {}", who, sig)),
    }
    if let Some(t) = text.strip_prefix("trap: ") {
        return Err(format!("{}, at the {}", suite::trap_said(t), who));
    }
    if let Some(e) = text.strip_prefix("error: ") {
        return Err(e.to_string());
    }
    match text.strip_prefix("side\n") {
        Some(t) => Side::read(t),
        None => Err(format!("the {} said nothing", who)),
    }
}

/// One case over a real wire: the server's child and then the client's,
/// the cut's table filled at both, the case's setters at the client, and
/// what each said. `func` is the case's twin
#[allow(clippy::too_many_arguments)]
pub fn real_run(module: &ssa::Module, jit: &JitCode, cut: &[(String, Vec<i64>)], setters: &[(String, Vec<i64>)], func: &str, args: &[i64], nrets: usize, delay: i64) -> Result<(Side, Side), String> {
    let (spid, spipe) = spawn(jit, |out| server_child(module, jit, cut, delay, out))?;
    let mut sread = std::io::BufReader::new(spipe);
    let mut first = String::new();
    sread.read_line(&mut first).map_err(|e| e.to_string())?;
    let port: Option<u16> = first.trim().parse().ok();
    let client = match port {
        Some(port) => {
            let (cpid, mut cpipe) = spawn(jit, |_| client_child(module, jit, port, cut, setters, func, args, nrets, delay))?;
            let mut text = String::new();
            let _ = cpipe.read_to_string(&mut text);
            Some(ended(cpid, text, "client"))
        }
        None => None,
    };
    let mut rest = String::new();
    let _ = sread.read_to_string(&mut rest);
    let server = ended(spid, if port.is_some() { rest } else { first + &rest }, "server");
    match client {
        Some(Ok(c)) => server.map(|s| (c, s)),
        Some(Err(e)) => Err(e),
        None => server.and(Err("the server said no port".into())),
    }
}

/// the two records joined (fm3 wire.md): each item a place put on with
/// when it arrived at the other by the other's clock, the set delay added,
/// and every line in the order of its first word, the sender's clock
pub fn joined(client: &Side, server: &Side, delay: i64) -> Vec<i64> {
    let mut rows: Vec<[i64; 6]> = Vec::new();
    for (side, other) in [(client, server), (server, client)] {
        let mut k = 0;
        for c in side.log.chunks(6).filter(|c| c.len() == 6) {
            let mut r = [c[0], c[1], c[2], c[3], c[4], c[5]];
            if ITEMS.contains(&c[1]) {
                r[3] = other.arrived.get(k).map_or(NEVER, |a| a + delay);
                k += 1;
            }
            rows.push(r);
        }
    }
    // (each place's lines of its homes at the end, 27, last, a stream at a time, as one process's record has them)
    let (mut homes, mut rest): (Vec<[i64; 6]>, Vec<[i64; 6]>) = rows.into_iter().partition(|r| r[1] == 27);
    rest.sort_by_key(|r| r[2]);
    homes.sort_by_key(|r| (r[3], r[0]));
    rest.into_iter().chain(homes).flatten().collect()
}

/// What step one of the real wire refuses, by name, each in one line that
/// says which step it is (fm3 wire.md, "the steps")
pub fn refused_step(then: bool, choose: bool, two: bool, fast: bool, other: bool) -> Option<String> {
    if fast {
        return Some("`--wire real` and `--fast` are not said together: a real wire runs on the real clock".into());
    }
    if then {
        return Some("a cut that moves while it runs is not run over a real wire in step one: it is step three's (fm3 wire.md)".into());
    }
    if choose {
        return Some("the chooser is not run over a real wire in step one: it is step three's (fm3 wire.md)".into());
    }
    if two {
        return Some("a case that names two people or more is three places, and three places over real wires are step three's (fm3 wire.md)".into());
    }
    if other {
        return Some("a wire with a speed, or one that changes or goes, is not a real wire's in step one: step two's (fm3 wire.md)".into());
    }
    None
}

/// What a store's build cannot carry over a real wire, by name. A function
/// partway whose record keeps an array or a string is carried (hop
/// sixty-three): what the continuation it goes on with reads that the
/// sender wrote crosses as its words and is laid out in the receiver's
/// arena (`land_vals_text`). Not where the server works ahead of a key:
/// before each turn ahead it copies every value its records hold, and a
/// record there holds the client's addresses for the client's functions
pub fn refused_build(ir: &str) -> Option<String> {
    if ir.contains("\nfn __act_land(") && !ir.contains("\nfn __act_land_at(") {
        return Some("a function partway here keeps an array or a string, and its record would carry an address in the sender's memory: not carried over a real wire in step one".into());
    }
    if ir.contains("\nfn __act_land(") && ir.contains("\nfn __wland_all(") {
        return Some("a function partway here keeps an array or a string, and the server works ahead of a key: before a turn ahead it copies every value its records hold, the client's among them, which are addresses in the client's memory; not carried over a real wire yet".into());
    }
    None
}

/// What a real wire needs of a store: its build for a wire that takes
/// time, with the queue's entries, and the module made from it
pub fn build(dir: &Path, policy: &ssa::Policy, level: usize) -> Result<(store::Store, lower::Lowered, ssa::Module, ssa::Policy), String> {
    let s = run::wired(dir)?;
    let policy = run::store_policy(&s, policy);
    let mut l = lower::lower(&s).map_err(|e| e.to_string())?;
    let placed: &Placed = l.placed.as_ref().ok_or("the store was not placed")?;
    let more = placed.real_text(&l.ir)?;
    l.ir.push_str(&more);
    if let Some(why) = refused_build(&l.ir) {
        return Err(why);
    }
    let module = run::build(&l.ir, &policy, level)?;
    Ok((s, l, module, policy))
}

/// the JIT of a module, as every native call has it
pub fn jit_of(module: &ssa::Module) -> Result<JitCode, String> {
    let enc = emit::Encoder::load("targets/arm64.encodings.json")?;
    emit::compile(module, &enc).and_then(|c| JitCode::new(&c))
}

/// A real run of one case at one cut, as the runner reads it: what the case
/// gave by its own times, each piece with when it was shown, the joined
/// record, and both sides
pub struct Ran {
    pub got: suite::Got,
    pub pieces: Vec<run::Shown>,
    pub log: Vec<i64>,
    pub client: Side,
    pub server: Side,
}

/// one case of `cases` at the cut `cut`, its setters those of `switches`
pub(super) fn case_run(module: &ssa::Module, jit: &JitCode, cut: &[(String, Vec<i64>)], p: &Planned, switches: &[(String, bool)], delay: i64) -> Result<Ran, String> {
    let c = &p.call;
    let setters = run::setters(switches, &c.input, &c.input_at);
    let mut table: Vec<(String, Vec<i64>)> = cut.to_vec();
    table.extend([("__wire_set".to_string(), vec![0]), ("__wire_but".to_string(), vec![0]), ("__wire_mode".to_string(), vec![0])]);
    let (client, server) = real_run(module, jit, &table, &setters, &c.func, &c.args, c.nrets, delay)?;
    let pieces = merged(run::shown_pieces(&client.text, &client.own, &client.shown));
    let log = joined(&client, &server, delay);
    let got = suite::Got { values: client.values.clone(), text: client.text.clone(), marks: client.own.clone() };
    Ok(Ran { got, pieces, log, client, server })
}

/// a time as the record of a real wire says it: microseconds where it is
/// under a millisecond
pub fn spell_us(us: i64) -> String {
    if us.abs() < 1000 {
        format!("{} µs", us)
    } else if us.abs() < 1_000_000 && us % 1000 != 0 {
        format!("{} ms", format!("{:.3}", us as f64 / 1000.0).trim_end_matches('0'))
    } else {
        run::spell_late(us)
    }
}

/// a moment as the record of a real wire says it: `177 µs`, `1.002031 s`
fn spell_at(us: i64) -> String {
    if us.abs() < 1_000_000 && us % 1000 != 0 { spell_us(us) } else { store::spell_time(us) }
}

/// The pieces of what was written, as over the simulated wire, but the
/// characters of one piece that arrived in rows of their own one piece,
/// shown when its last arrived: over a real wire two rows a microsecond
/// apart are two moments
fn merged(pieces: Vec<run::Shown>) -> Vec<run::Shown> {
    let mut out: Vec<run::Shown> = Vec::new();
    for p in pieces {
        match out.last_mut() {
            Some(l) if l.own == p.own && !l.text.ends_with('\n') => {
                l.text.push_str(&p.text);
                l.shown = l.shown.max(p.shown);
            }
            _ => out.push(p),
        }
    }
    out
}

/// the median of the pings' round trips, in microseconds, halved: a row's
/// one-way time on the loopback
pub fn one_way(pings: &[i64]) -> i64 {
    let mut v = pings.to_vec();
    v.sort();
    v.get(v.len() / 2).map_or(0, |n| n / 2000)
}

/// what the wire carried, as `run` says it beneath what was late
pub fn carried_said(ran: &Ran) -> String {
    let (c, s) = (&ran.client, &ran.server);
    format!("the socket: client → server {} rows, {} bytes; server → client {} rows, {} bytes; a row one way about {} (half the middle of {} round trips); the case took {} at the client\n", c.rows_out, c.bytes_out, s.rows_out, s.bytes_out, spell_us(one_way(&c.pings)), c.pings.len(), spell_us(c.took))
}

/// `probe zero <store> run '<case>' --server <cut> --wire real`: the case
/// at that cut with the client and the server in two processes, by its own
/// times, which is what is compared with the case; beneath it what was
/// late, what the socket carried, and what the home stamped
pub fn run_real(dir: &Path, which: &str, policy: &ssa::Policy, level: usize, server: Option<&str>, real: Real, dropped: Option<&str>) -> Result<String, String> {
    let (s, l, module, policy) = build(dir, policy, level)?;
    let placed = l.placed.as_ref().ok_or("the store was not placed")?;
    let (first, _) = run::cuts_named(placed, server, &[])?;
    if let Some(why) = placed.refused(&first) {
        return Err(format!("this cut cannot be run yet: {}", why));
    }
    let cut = placed.cut_but(&first, dropped);
    let calls = run::calls_of(&s, &l, &policy)?;
    if run::case_named(&calls, which).is_none() && run::names_two(dir, which) {
        return Err(refused_step(false, false, true, false, false).unwrap());
    }
    let p = run::case_named(&calls, which).ok_or_else(|| format!("no case '{}' in the store's ## testing sections", which))?;
    let jit = jit_of(&module)?;
    let ran = case_run(&module, &jit, &cut.before, p, &p.call.context, real.delay)?;
    let shown = match p.call.expect {
        store::Expect::Timed(_) => store::spell_timed(&run::pieces(&ran.got)),
        store::Expect::Text(_) => format!("{:?}", ran.got.text.strip_suffix('\n').unwrap_or(&ran.got.text)),
        _ => ran.got.values.iter().map(|v| v.to_string()).collect::<Vec<_>>().join(", "),
    };
    let thrown = if run::works_ahead(&l.ir) { run::thrown_said(Some(ran.server.thrown)) } else { String::new() };
    Ok(format!("{}\n{}, a {} wire: the client and the server in two processes on this machine, over the loopback\n→ {}\n{}{}{}{}", p.text.split('→').next().unwrap_or("").trim(), cut.label, real.said(), shown, late_said_us(&ran.pieces), carried_said(&ran), thrown, run::homes_end_said(&placed.homes, &ran.log)))
}

/// what was late, as `late_said` says it, a time under a millisecond in
/// microseconds
pub fn late_said_us(pieces: &[run::Shown]) -> String {
    let late: Vec<&run::Shown> = pieces.iter().filter(|p| p.shown > p.own).collect();
    if late.is_empty() {
        return format!("late: nothing, of {} piece{}\n", pieces.len(), if pieces.len() == 1 { "" } else { "s" });
    }
    let most = late.iter().map(|p| p.shown - p.own).max().unwrap_or(0);
    let mut out = format!("late: {} piece{} of {}, the most {}\n", late.len(), if late.len() == 1 { "" } else { "s" }, pieces.len(), spell_us(most));
    for p in late {
        out.push_str(&format!("  {:?} for {}, shown at {}, {} late\n", p.text, store::spell_time(p.own), spell_at(p.shown), spell_us(p.shown - p.own)));
    }
    out
}

/// the stores of step one (fm3 wire.md): the touchstone's and hop
/// fifty-eight's, and from hop sixty-three the triangle's, its frame and
/// its spin, which `probe zero test <dir> --wire real` runs where it
/// finds them under the directory
pub const STEP_ONE: [&str; 5] = ["hello", "restart", "stamp", "counter", "triangle"];

/// what the suite over a real wire found
#[derive(Default)]
pub struct Reals {
    pub stores: usize,
    pub cuts: usize,
    pub refused: usize,
    pub passed: usize,
    pub failed: usize,
    pub stamped: usize,
    pub most: i64,
    pub pieces: usize,
    pub late: usize,
    pub one_way: Vec<i64>,
}

/// One store over a real wire (fm3 wire.md, step one): its build for a wire
/// with the queue's entries, compiled once, and each case in each of its
/// contexts at every cut the runner tries, every unit on the server and
/// each alone, the client and the server in two processes; a case compared
/// by its own times as over the simulated wire, what was late counted. The
/// runs of a store go a few at a time, each its own pair of processes
pub fn test_real(sdir: &Path, policy: &ssa::Policy, level: usize, report: &mut Report, reals: &mut Reals, delay: i64) {
    let name = sdir.file_name().unwrap().to_string_lossy().to_string();
    let (s, l, module, policy) = match build(sdir, policy, level) {
        Ok(b) => b,
        Err(e) => {
            report.failed += 1;
            report.log.push_str(&format!("FAIL  {:<16} not built for a real wire: {}\n", name, e));
            return;
        }
    };
    reals.stores += 1;
    let Some(placed) = &l.placed else { return };
    run::two_named(&s, "real", &name, report);
    let built = (|| -> Result<(Vec<Planned>, Vec<run::Run>), String> {
        let cases = run::calls_of(&s, &l, &policy)?;
        let (runs, _) = run::plan(&s, &cases)?;
        Ok((cases, runs))
    })();
    let (cases, runs) = match built {
        Ok(b) => b,
        Err(e) => {
            report.failed += 1;
            report.log.push_str(&format!("FAIL  {:<16} {}\n", name, e));
            return;
        }
    };
    let unreached = run::out_of_reach(&module, &l.funcs, run::kind_of(suite::Backend::Native));
    let runs: Vec<&run::Run> = runs.iter().filter(|r| !unreached.contains_key(&cases[r.case].call.func)).collect();
    let jit = match jit_of(&module) {
        Ok(j) => j,
        Err(e) => {
            report.failed += 1;
            report.log.push_str(&format!("FAIL  {:<16} {}\n", name, e));
            return;
        }
    };
    let (mut ran, mut refused, mut failed, mut stamped, mut most) = (0, 0, 0, 0, 0i64);
    for server in super::places::cuts(placed) {
        let label = placed.label(&server);
        if let Some(why) = placed.refused(&server) {
            refused += 1;
            reals.refused += 1;
            report.log.push_str(&format!("real  {:<16} [{}] refused: {}\n", name, label, why));
            continue;
        }
        reals.cuts += 1;
        ran += 1;
        let cut = placed.cut(&server);
        // (a few at a time: each run is two processes that mostly wait on the clock)
        let results = parallel(&runs, |r| case_run(&module, &jit, &cut.before, &cases[r.case], &r.switches, delay));
        for (r, got) in runs.iter().zip(results) {
            let text = format!("{} [{}, a {} wire]", run::labelled(&cases[r.case].text, &r.label), label, Real { delay }.said());
            let (own, stamp) = match got {
                Ok(ran) => {
                    most = most.max(ran.pieces.iter().map(|p| p.shown - p.own).max().unwrap_or(0));
                    reals.pieces += ran.pieces.len();
                    reals.late += ran.pieces.iter().filter(|p| p.shown > p.own).count();
                    reals.one_way.push(one_way(&ran.client.pings));
                    let stamp = run::home_stamped(&ran.log);
                    let mut g = ran.got;
                    if !matches!(cases[r.case].call.expect, store::Expect::Timed(_)) {
                        g.marks.clear();
                    }
                    (Ok(g), stamp)
                }
                Err(e) => (Err(e), false),
            };
            let (ok, note) = run::judge(&cases[r.case].call.expect, own);
            if ok {
                reals.passed += 1;
                report.passed += 1;
            } else if stamp {
                // (as over the simulated wire, question 180: the home's clock stamped later than the item's own time, and the case reads it)
                stamped += 1;
                reals.stamped += 1;
                report.log.push_str(&format!("home  {:<16} {}   {}: the home stamped an item by its own clock, and the case reads it: not compared, as over the simulated wire\n", name, text, note));
            } else {
                failed += 1;
                reals.failed += 1;
                report.failed += 1;
                report.log.push_str(&format!("FAIL  {:<16} {}   {}\n", name, text, note));
            }
        }
    }
    reals.most = reals.most.max(most);
    report.log.push_str(&format!("reals {:<16} {} units, {} cuts run{}{}\n", name, placed.units.len(), ran, if refused > 0 { format!(", {} refused", refused) } else { String::new() }, if failed > 0 { format!(", {} runs do not hold", failed) } else if stamped > 0 { format!(": holds by its own times; its stamps not compared, {} run{}; the most a piece is late {}", stamped, if stamped == 1 { "" } else { "s" }, spell_us(most)) } else { format!(": every case holds at each by its own times over the socket; the most a piece is late {}", spell_us(most)) }));
}

/// the runs of one cut, `WIDTH` at a time, each in a thread of its own
/// that forks its two places; their results in order
fn parallel<'a, F: Fn(&run::Run) -> Result<Ran, String> + Sync>(runs: &[&'a run::Run], f: F) -> Vec<Result<Ran, String>> {
    const WIDTH: usize = 6;
    let mut out: Vec<Result<Ran, String>> = Vec::new();
    for chunk in runs.chunks(WIDTH) {
        let got: Vec<Result<Ran, String>> = std::thread::scope(|sc| {
            let hs: Vec<_> = chunk.iter().map(|r| sc.spawn(|| f(r))).collect();
            hs.into_iter().map(|h| h.join().unwrap_or_else(|_| Err("a runner thread panicked".into()))).collect()
        });
        out.extend(got);
    }
    out
}

/// `probe zero test <dir> --wire real`: the stores of step one under the
/// directory, each over a real wire at every cut
pub fn test(dir: &Path, level: usize, delay: i64) -> Result<Report, String> {
    let policy = suite::backend_policy(suite::Backend::Native)?;
    let mut report = Report { passed: 0, failed: 0, skipped: 0, log: String::new() };
    let mut reals = Reals::default();
    let found: Vec<std::path::PathBuf> = STEP_ONE.iter().map(|n| dir.join(n)).filter(|p| p.is_dir()).collect();
    if found.is_empty() {
        return Err(format!("none of step one's stores ({}) is under {}", STEP_ONE.join(", "), dir.display()));
    }
    for sdir in &found {
        test_real(sdir, &policy, level, &mut report, &mut reals, delay);
    }
    let mut ow = reals.one_way.clone();
    ow.sort();
    report.log.push_str(&format!("\nreal wire{}: {} stores, {} cuts run, {} refused; {} of {} runs at a cut hold by their own times{}; {} of {} pieces of text shown late, the most {}; a row one way on the loopback {} to {}, the middle {}\n", if delay > 0 { format!(", {}", run::spell_late(delay)) } else { String::new() }, reals.stores, reals.cuts, reals.refused, reals.passed, reals.passed + reals.failed, if reals.stamped > 0 { format!(", {} not compared (a home's stamp)", reals.stamped) } else { String::new() }, reals.late, reals.pieces, spell_us(reals.most), spell_us(*ow.first().unwrap_or(&0)), spell_us(*ow.last().unwrap_or(&0)), spell_us(*ow.get(ow.len() / 2).unwrap_or(&0))));
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::opt;

    /// hello at one cut over a real wire (fm3 wire.md, step one): the
    /// countdown on the server in a process of its own, the client in
    /// another, every row across a loopback socket; the case's own line by
    /// its own times, every piece shown within 10 ms of its time, and both
    /// processes ended with the case
    #[test]
    fn a_real_wire_runs_hello() {
        let native = suite::backend_policy(suite::Backend::Native).unwrap();
        let out = run_real(Path::new("suite/zero/hello"), "run()", &native, opt::MAX_LEVEL, Some("count down"), Real::default(), None).unwrap();
        assert!(out.contains("\n→ \"10\\n9\\n8\\n7\\n6\\n5\\n4\\n3\\n2\\n1\\n\" at 1 hz, \"hello world\\ngoodbye\" at 10 s\n"), "{}", out);
        assert!(out.contains("server: count down, a real wire"), "{}", out);
        let most = out.lines().find_map(|l| l.strip_prefix("late: ")).unwrap_or("");
        assert!(most.starts_with("nothing") || most.ends_with(" µs") || most.split(" the most ").nth(1).is_some_and(|m| m.ends_with(" ms") && m.trim_end_matches(" ms").parse::<f64>().is_ok_and(|v| v < 10.0)), "{}", out);
        assert!(out.contains("the socket: client → server ") && out.contains("; server → client "), "{}", out);
    }

    /// Memory of its own, really (fm3 wire.md): with one carried field
    /// dropped from the cut, which only a test can ask for, the server's
    /// process has only its own memory to read, and the case goes wrong.
    /// hello's countdown switched off at the client, its switch not carried
    /// to the server: the server counts down all the same
    #[test]
    fn a_real_wire_has_memory_of_its_own() {
        let native = suite::backend_policy(suite::Backend::Native).unwrap();
        let hello = Path::new("suite/zero/hello");
        let case = "run() with countdown off";
        let right = run_real(hello, case, &native, opt::MAX_LEVEL, Some("all"), Real::default(), None).unwrap();
        assert!(right.contains("\n→ \"hello world\\ngoodbye\"\n"), "{}", right);
        let wrong = run_real(hello, case, &native, opt::MAX_LEVEL, Some("all"), Real::default(), Some("__on_countdown")).unwrap();
        assert!(wrong.contains("\n→ \"10\\n9\\n8\\n7\\n6\\n5\\n4\\n3\\n2\\n1\\nhello world\\ngoodbye\"\n"), "{}", wrong);
    }

    /// what step one refuses, each by name in one line that says which
    /// step it is (fm3 wire.md, "the steps")
    #[test]
    fn step_one_refuses_by_name() {
        let native = suite::backend_policy(suite::Backend::Native).unwrap();
        assert!(refused_step(true, false, false, false, false).unwrap().contains("step three's"));
        assert!(refused_step(false, true, false, false, false).unwrap().contains("step three's"));
        assert!(refused_step(false, false, true, false, false).unwrap().contains("step three's"));
        assert!(refused_step(false, false, false, true, false).unwrap().contains("real clock"));
        assert!(refused_step(false, false, false, false, true).unwrap().contains("step two's"));
        assert_eq!(refused_step(false, false, false, false, false), None);
        let two = run_real(Path::new("suite/zero/counter"), "run() with ann in \"k\" at 1 s, bob in \"k\" at 1 s", &native, opt::MAX_LEVEL, Some("all"), Real::default(), None).unwrap_err();
        assert!(two.contains("three places") && two.contains("step three's"), "{}", two);
        assert!(refused_build("\nfn __act_land(row: index)\n").is_some() && refused_build("\nfn __world()\n").is_none());
    }

    /// a set delay on a real wire (fm3 wire.md, step one): each row held
    /// 100 ms where it arrives, so hello with everything on the server shows
    /// its first piece a round trip late, as over the simulated wire, and
    /// the loopback's own time beside it
    #[test]
    fn a_real_wire_holds_a_set_delay() {
        let native = suite::backend_policy(suite::Backend::Native).unwrap();
        let out = run_real(Path::new("suite/zero/hello"), "run()", &native, opt::MAX_LEVEL, Some("all"), Real { delay: 100_000 }, None).unwrap();
        assert!(out.contains("server: all, a real, 100 ms wire") && out.contains("\n→ \"10\\n9\\n8\\n7\\n6\\n5\\n4\\n3\\n2\\n1\\n\" at 1 hz, \"hello world\\ngoodbye\" at 10 s\n"), "{}", out);
        let late = out.lines().find(|l| l.starts_with("  \"10\\n\" for 0 s, shown at ")).unwrap_or("");
        let ms: f64 = late.rsplit(", ").next().unwrap_or("").trim_end_matches(" late").trim_end_matches(" ms").parse().unwrap_or(0.0);
        assert!((200.0..210.0).contains(&ms), "{}", out);
    }
}
