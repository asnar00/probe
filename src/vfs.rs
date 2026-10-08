//! Where the compiler reads from: the disk, or texts handed to it.
//!
//! Everything on the way from a store's text to an emitted module reads
//! through here: a store's folders (`zero/store.rs`), the IR's own
//! libraries (`ssa::with_prelude`), a target's learned encodings
//! (`emit_wasm::WEncoder::load`) and its platform file
//! (`platform::Platform::load_named`). On a machine with a disk nothing
//! is mounted and each call is `std::fs`'s. Where there is no disk, the
//! compiler built for `wasm32` and run in a page, the host mounts the
//! texts first, and from the first `mount` on this thread every read is
//! of what was mounted and of nothing else.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};

thread_local! {
    static MOUNTED: RefCell<Option<BTreeMap<String, String>>> = const { RefCell::new(None) };
}

fn key(path: &Path) -> String {
    let s = path.to_string_lossy().replace('\\', "/");
    s.trim_start_matches("./").trim_end_matches('/').to_string()
}

/// hand the compiler a file's text under a path; from the first call on,
/// this thread reads only what was mounted
#[allow(dead_code)]
pub fn mount(path: &str, text: String) {
    MOUNTED.with(|m| m.borrow_mut().get_or_insert_with(BTreeMap::new).insert(key(Path::new(path)), text));
}

/// forget every mounted file under a path, a store's folder say
#[allow(dead_code)]
pub fn unmount(prefix: &str) {
    let p = format!("{}/", key(Path::new(prefix)));
    MOUNTED.with(|m| {
        if let Some(map) = m.borrow_mut().as_mut() {
            map.retain(|k, _| !k.starts_with(&p));
        }
    });
}

fn missing(path: &Path) -> io::Error {
    io::Error::new(io::ErrorKind::NotFound, format!("{}: no such file", path.display()))
}

/// `std::fs::read_to_string`, or the mounted text
pub fn read_to_string(path: impl AsRef<Path>) -> io::Result<String> {
    let path = path.as_ref();
    MOUNTED.with(|m| match m.borrow().as_ref() {
        Some(map) => map.get(&key(path)).cloned().ok_or_else(|| missing(path)),
        None => std::fs::read_to_string(path),
    })
}

/// the paths directly inside a directory, as `std::fs::read_dir` gives
/// them and in no promised order
pub fn read_dir(dir: impl AsRef<Path>) -> io::Result<Vec<PathBuf>> {
    let dir = dir.as_ref();
    MOUNTED.with(|m| match m.borrow().as_ref() {
        Some(map) => {
            let p = format!("{}/", key(dir));
            let mut out: Vec<PathBuf> = Vec::new();
            for k in map.keys().filter(|k| k.starts_with(&p)) {
                let first = k[p.len()..].split('/').next().unwrap_or("");
                let child = PathBuf::from(format!("{}{}", p, first));
                if !first.is_empty() && !out.contains(&child) {
                    out.push(child);
                }
            }
            if out.is_empty() { Err(missing(dir)) } else { Ok(out) }
        }
        None => Ok(std::fs::read_dir(dir)?.filter_map(|e| e.ok().map(|e| e.path())).collect()),
    })
}

/// is this path a directory: on the disk, or one that mounted files lie under
pub fn is_dir(path: impl AsRef<Path>) -> bool {
    let path = path.as_ref();
    MOUNTED.with(|m| match m.borrow().as_ref() {
        Some(map) => {
            let p = format!("{}/", key(path));
            map.keys().any(|k| k.starts_with(&p))
        }
        None => path.is_dir(),
    })
}

/// is there a file here
pub fn exists(path: impl AsRef<Path>) -> bool {
    let path = path.as_ref();
    MOUNTED.with(|m| match m.borrow().as_ref() {
        Some(map) => map.contains_key(&key(path)) || is_dir(path),
        None => path.exists(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// mounted texts are read back, listed by folder, and stand in for
    /// the disk from the first mount on
    #[test]
    fn mounted_texts_stand_in_for_the_disk() {
        std::thread::spawn(|| {
            mount("store/a/a.zero", "one".into());
            mount("store/a/a.md", "two".into());
            mount("store/order.md", "three".into());
            assert_eq!(read_to_string("store/a/a.zero").unwrap(), "one");
            assert!(read_to_string("Cargo.toml").is_err());
            let mut inside = read_dir("store").unwrap();
            inside.sort();
            assert_eq!(inside, [PathBuf::from("store/a"), PathBuf::from("store/order.md")]);
            assert!(is_dir("store/a") && !is_dir("store/order.md") && exists("store/order.md"));
            unmount("store");
            assert!(read_dir("store").is_err());
        })
        .join()
        .unwrap();
    }
}
