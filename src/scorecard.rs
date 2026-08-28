//! The scorecard: every learned template checked against the official
//! instruction inventory of its target, and the inventory checked for
//! what has not been learned.
//!
//! The learner never reads these tables — it derives encodings from an
//! assembler's output alone. They are the independent referee afterwards:
//! for a learned template, the official encoding its fixed word decodes
//! to must exist, the learned fields must sit inside that encoding's
//! operand fields, and (wasm) the opcode's name must be the template's.
//! tools/get-isa-tables.sh fetches the three tables.

use std::collections::BTreeMap;
use std::fmt::Write;

/// one official encoding: which words it claims, its operand fields
struct Official {
    name: String,
    /// the mnemonic, for counting: an instruction has several encodings
    mnemonic: String,
    group: String,
    mask: u64,
    value: u64,
    fields: Vec<(String, Vec<u32>)>,
}

impl Official {
    fn matches(&self, word: u64) -> bool {
        word & self.mask == self.value
    }
    fn field_of(&self, bits: &[u32]) -> Option<String> {
        // the named fields the learned bits fall in, all of them inside
        let names: Vec<&str> = self
            .fields
            .iter()
            .filter(|(_, fb)| bits.iter().any(|b| fb.contains(b)))
            .map(|(n, _)| n.as_str())
            .collect();
        let covered = bits.iter().all(|b| self.fields.iter().any(|(_, fb)| fb.contains(b)));
        covered.then(|| names.join("+"))
    }
}

/// a learned template, reduced to what the scorecard compares
struct Learned {
    template: String,
    word: u64,
    fields: Vec<(String, Vec<u32>)>,
}

// ---------------------------------------------------------------- JSON

/// the learned encodings file, read with the same minimal parser the
/// emitters use
fn learned(target: &str) -> Result<Vec<Learned>, String> {
    let path = format!("targets/{}.encodings.json", target);
    let text = std::fs::read_to_string(&path).map_err(|e| format!("{}: {}", path, e))?;
    let json = crate::emit::parse_json_pub(&text)?;
    let mut out = Vec::new();
    for inst in json.get("instructions").and_then(|v| v.a()).ok_or("no instructions")? {
        let template = inst.get("template").and_then(|v| v.s()).unwrap_or("").to_string();
        let mut fields = Vec::new();
        let word = if target == "wasm32" {
            // the first fixed piece: the opcode byte, or a prefix and a uleb
            let pieces = inst.get("pieces").and_then(|v| v.a()).ok_or("no pieces")?;
            let hex = pieces[0].get("fixed").and_then(|v| v.s()).unwrap_or("");
            let bytes: Vec<u8> = (0..hex.len() / 2).map(|i| u8::from_str_radix(&hex[2 * i..2 * i + 2], 16).unwrap()).collect();
            match bytes.first() {
                Some(&p @ (0xfc | 0xfd | 0xfe)) => {
                    let mut code = 0u64;
                    for (i, b) in bytes[1..].iter().enumerate() {
                        code |= ((b & 0x7f) as u64) << (7 * i);
                        if b & 0x80 == 0 {
                            break;
                        }
                    }
                    (p as u64) << 32 | code
                }
                Some(&b) => b as u64,
                None => 0,
            }
        } else {
            let fixed = inst.get("fixed").and_then(|v| v.s()).unwrap_or("0x0");
            u64::from_str_radix(fixed.trim_start_matches("0x"), 16).map_err(|e| e.to_string())?
        };
        if let Some(fs) = inst.get("fields").and_then(|v| v.a()) {
            for f in fs {
                let slot = f.get("slot").and_then(|v| v.s()).unwrap_or("?").to_string();
                let bits: Vec<u32> = match f.get("bits").and_then(|v| v.a()) {
                    Some(bs) => bs.iter().filter_map(|b| b.n()).map(|b| b as u32).collect(),
                    // a table field: every bit any entry touches
                    None => {
                        let mut m = 0u64;
                        for e in f.get("entries").and_then(|v| v.a()).into_iter().flatten() {
                            m |= u64::from_str_radix(e.s().unwrap_or("0").trim_start_matches("0x"), 16).unwrap_or(0);
                        }
                        (0..64).filter(|b| m >> b & 1 == 1).collect()
                    }
                };
                fields.push((slot, bits));
            }
        }
        out.push(Learned { template, word, fields });
    }
    Ok(out)
}

// ---------------------------------------------------------------- arm64

/// the A64 ISA XML: one file per instruction, `<iclass>`es with a
/// register diagram of `<box>`es whose `<c>` cells are settled bits or
/// blanks, and `<encoding>`s that settle more boxes and name a mnemonic
fn arm64_official(dir: &str) -> Result<Vec<Official>, String> {
    let mut out = Vec::new();
    let mut entries: Vec<_> = std::fs::read_dir(dir).map_err(|e| format!("{}: {}", dir, e))?.flatten().collect();
    entries.sort_by_key(|e| e.file_name());
    for entry in entries {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("xml") {
            continue;
        }
        let text = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
        if !text.contains("<instructionsection") || text.contains("type=\"pseudocode\"") {
            continue;
        }
        let file = path.file_stem().unwrap().to_string_lossy().to_string();
        for iclass in split_elements(&text, "iclass") {
            let Some(diagram) = split_elements(iclass, "regdiagram").into_iter().next() else { continue };
            if !diagram.contains("form=\"32\"") {
                continue;
            }
            let base = boxes(diagram);
            for enc in split_elements(iclass, "encoding") {
                let name = attr(enc, "encoding", "name").unwrap_or_else(|| file.clone());
                let mut settled = base.clone();
                for (hi, w, n, cells) in boxes(enc) {
                    // an encoding's box overrides the class's
                    settled.retain(|(h, ..)| *h != hi);
                    settled.push((hi, w, n, cells));
                }
                let (mut mask, mut value, mut fields) = (0u64, 0u64, Vec::new());
                for (hi, w, n, cells) in &settled {
                    let mut free = Vec::new();
                    for (k, c) in cells.iter().enumerate() {
                        let bit = hi - k as u32;
                        match c.as_str() {
                            "0" | "(0)" => mask |= 1 << bit,
                            "1" | "(1)" => {
                                mask |= 1 << bit;
                                value |= 1 << bit
                            }
                            _ => free.push(bit),
                        }
                    }
                    let _ = w;
                    if !free.is_empty() {
                        fields.push((n.clone().unwrap_or_else(|| format!("bit{}", hi)), free));
                    }
                }
                let group = docvar(enc, "instr-class").or_else(|| docvar(iclass, "instr-class")).unwrap_or_else(|| "?".into());
                let mnemonic = docvar(enc, "mnemonic").unwrap_or_else(|| name.clone()).to_lowercase();
                out.push(Official { name, mnemonic, group, mask, value, fields });
            }
        }
    }
    Ok(out)
}

/// the text of every `<tag ...>...</tag>` element, outermost only
fn split_elements<'a>(text: &'a str, tag: &str) -> Vec<&'a str> {
    let open = format!("<{}", tag);
    let close = format!("</{}>", tag);
    let mut out = Vec::new();
    let mut pos = 0;
    while let Some(s) = text[pos..].find(&open) {
        let start = pos + s;
        // `<tag` must be followed by a space or `>`
        let after = text.as_bytes().get(start + open.len()).copied();
        if after != Some(b' ') && after != Some(b'>') {
            pos = start + open.len();
            continue;
        }
        let Some(e) = text[start..].find(&close) else { break };
        out.push(&text[start..start + e + close.len()]);
        pos = start + e + close.len();
    }
    out
}

fn attr(elem: &str, tag: &str, name: &str) -> Option<String> {
    let head_end = elem.find('>')?;
    let head = &elem[..head_end];
    if !head.starts_with(&format!("<{}", tag)) {
        return None;
    }
    let key = format!(" {}=\"", name);
    let s = head.find(&key)? + key.len();
    let e = head[s..].find('"')?;
    Some(head[s..s + e].to_string())
}

fn docvar(elem: &str, key: &str) -> Option<String> {
    let k = format!("<docvar key=\"{}\" value=\"", key);
    let s = elem.find(&k)? + k.len();
    let e = elem[s..].find('"')?;
    Some(elem[s..s + e].to_string())
}

/// the `<box>`es of an element, as (hibit, width, name, cells) — a cell
/// per bit, expanded from `colspan`
fn boxes(elem: &str) -> Vec<(u32, u32, Option<String>, Vec<String>)> {
    let mut out = Vec::new();
    for b in split_elements(elem, "box") {
        let hi: u32 = attr(b, "box", "hibit").and_then(|v| v.parse().ok()).unwrap_or(0);
        let width: u32 = attr(b, "box", "width").and_then(|v| v.parse().ok()).unwrap_or(1);
        let name = attr(b, "box", "name");
        let mut cells = Vec::new();
        for c in split_elements(b, "c") {
            let span: usize = attr(c, "c", "colspan").and_then(|v| v.parse().ok()).unwrap_or(1);
            let inner = c[c.find('>').unwrap() + 1..c.rfind("</c>").unwrap()].trim().to_string();
            for _ in 0..span {
                cells.push(inner.clone());
            }
        }
        while (cells.len() as u32) < width {
            cells.push(String::new());
        }
        out.push((hi, width, name, cells));
    }
    out
}

// ---------------------------------------------------------------- riscv64

/// riscv-opcodes: `name args... hi..lo=val bit=val`, argument positions
/// from arg_lut.csv; `$pseudo_op` lines are aliases with a parent
fn riscv64_official(dir: &str, exts: &[&str]) -> Result<Vec<Official>, String> {
    let lut_text = std::fs::read_to_string(format!("{}/arg_lut.csv", dir)).map_err(|e| e.to_string())?;
    let mut lut: BTreeMap<String, (u32, u32)> = BTreeMap::new();
    for line in lut_text.lines() {
        let f: Vec<&str> = line.split(',').map(|s| s.trim().trim_matches('"')).collect();
        if f.len() == 3 {
            lut.insert(f[0].to_string(), (f[1].parse().unwrap(), f[2].parse().unwrap()));
        }
    }
    let mut out = Vec::new();
    for ext in exts {
        let text = std::fs::read_to_string(format!("{}/extensions/{}", dir, ext)).map_err(|e| format!("{}: {}", ext, e))?;
        for line in text.lines() {
            let line = line.split('#').next().unwrap().trim();
            if line.is_empty() {
                continue;
            }
            let mut toks: Vec<&str> = line.split_whitespace().collect();
            let mut name = toks.remove(0).to_string();
            if name == "$pseudo_op" {
                let parent = toks.remove(0);
                name = format!("{} ({})", toks.remove(0), parent.split("::").last().unwrap());
            } else if name == "$import" {
                continue;
            }
            let (mut mask, mut value, mut fields) = (0u64, 0u64, Vec::new());
            for t in toks {
                if let Some((range, v)) = t.split_once('=') {
                    // `rs2=rs1` in a pseudo-op ties fields; not a fixed bit
                    let (hi, lo) = match range.split_once("..") {
                        Some((h, l)) => match (h.parse::<u32>(), l.parse::<u32>()) {
                            (Ok(h), Ok(l)) => (h, l),
                            _ => continue,
                        },
                        None => match range.parse::<u32>() {
                            Ok(b) => (b, b),
                            Err(_) => continue,
                        },
                    };
                    let v = if let Some(h) = v.strip_prefix("0x") { u64::from_str_radix(h, 16).unwrap() } else if let Ok(n) = v.parse::<u64>() { n } else { continue };
                    let m = ((1u64 << (hi - lo + 1)) - 1) << lo;
                    mask |= m;
                    value |= (v << lo) & m;
                } else if let Some(&(hi, lo)) = lut.get(t) {
                    fields.push((t.to_string(), (lo..=hi).collect()));
                }
            }
            let mnemonic = name.split(' ').next().unwrap().to_string();
            out.push(Official { name, mnemonic, group: ext.to_string(), mask, value, fields });
        }
    }
    Ok(out)
}

// ---------------------------------------------------------------- wasm32

/// wabt's opcode.def: WABT_OPCODE(rtype, t1, t2, t3, mem, memmask,
/// prefix, code, Name, "text", decomp)
fn wasm32_official(path: &str) -> Result<Vec<Official>, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {}", path, e))?;
    let mut out = Vec::new();
    for line in text.lines() {
        let Some(rest) = line.trim().strip_prefix("WABT_OPCODE(") else { continue };
        let f: Vec<&str> = rest.trim_end_matches(')').split(',').map(|s| s.trim()).collect();
        if f.len() < 11 {
            continue;
        }
        let num = |s: &str| -> u64 { s.strip_prefix("0x").map(|h| u64::from_str_radix(h, 16).unwrap()).unwrap_or_else(|| s.parse().unwrap()) };
        let (prefix, code) = (num(f[6]), num(f[7]));
        let name = f[9].trim_matches('"').to_string();
        // wabt's own interpreter opcodes (Interp*) are not WebAssembly
        if name.is_empty() || f[8].starts_with("Interp") {
            continue;
        }
        let group = match prefix {
            0 => "core",
            0xfc => "0xfc (saturating conversions, bulk memory)",
            0xfd => "0xfd (simd)",
            0xfe => "0xfe (threads)",
            _ => "other",
        }
        .to_string();
        let word = if prefix == 0 { code } else { prefix << 32 | code };
        out.push(Official { mnemonic: name.clone(), name, group, mask: u64::MAX, value: word, fields: Vec::new() });
    }
    Ok(out)
}

// ---------------------------------------------------------------- the card

pub struct Card {
    pub text: String,
    pub problems: usize,
}

pub fn scorecard(target: &str) -> Result<Card, String> {
    let (official, focus): (Vec<Official>, Vec<&str>) = match target {
        "arm64" => (arm64_official("tools/ISA_A64_xml_A_profile-2022-12")?, vec!["general", "float"]),
        "riscv64" => {
            let exts = ["rv_i", "rv64_i", "rv_m", "rv64_m", "rv_a", "rv64_a", "rv_f", "rv64_f", "rv_d", "rv64_d", "rv_v", "rv_zicsr", "rv_zifencei", "rv_system"];
            (riscv64_official("tools/riscv-opcodes", &exts)?, exts.to_vec())
        }
        "wasm32" => (wasm32_official("tools/wabt/include/wabt/opcode.def")?, vec!["core", "0xfc (saturating conversions, bulk memory)"]),
        t => return Err(format!("no scorecard for '{}'", t)),
    };
    let learned = learned(target)?;
    let mut text = String::new();
    let mut problems = 0;
    let _ = writeln!(text, "# {} encoding scorecard\n", target);
    let _ = writeln!(text, "Generated by `probe scorecard {}` from `targets/{}.encodings.json` against\nthe official table (`tools/get-isa-tables.sh`). The learner never reads the\ntable; this is the check afterwards.\n", target, target);

    // 1. every learned template against the table
    let _ = writeln!(text, "## Learned templates: {}\n", learned.len());
    let _ = writeln!(text, "| template | official encoding | fields |\n|---|---|---|");
    let mut matched_officials: Vec<usize> = Vec::new();
    for l in &learned {
        let hits: Vec<usize> = (0..official.len()).filter(|&i| official[i].matches(l.word)).collect();
        if hits.is_empty() {
            problems += 1;
            let _ = writeln!(text, "| `{}` | **no official encoding matches 0x{:x}** | |", l.template, l.word);
            continue;
        }
        let names: Vec<String> = hits.iter().map(|&i| official[i].name.clone()).collect();
        // the fields, against the first (non-alias where possible) hit
        let primary = *hits.iter().find(|&&i| !official[i].name.contains('(')).unwrap_or(&hits[0]);
        matched_officials.extend(&hits);
        let mut cols = Vec::new();
        let mut bad = false;
        for (slot, bits) in &l.fields {
            match official[primary].field_of(bits) {
                Some(f) => cols.push(format!("`{}`→{}", slot, f)),
                None => {
                    bad = true;
                    cols.push(format!("`{}`→**bits {:?} are not an operand field**", slot, bits));
                }
            }
        }
        if target == "wasm32" {
            let mnemonic = l.template.split_whitespace().next().unwrap_or("");
            if !names.iter().any(|n| n == mnemonic) {
                bad = true;
                cols.push(format!("**opcode is `{}`, not `{}`**", names.join("/"), mnemonic));
            }
        }
        if bad {
            problems += 1;
        }
        let _ = writeln!(text, "| `{}` | {} | {} |", l.template, names.join(", "), cols.join(", "));
    }
    let _ = writeln!(text, "\n{} of {} templates match an official encoding with their fields inside its operand fields; {} problem(s).\n",
        learned.len() - problems, learned.len(), problems);

    // 2. the inventory, by group: what is not learned yet
    let _ = writeln!(text, "## Coverage of the official inventory\n");
    let _ = writeln!(text, "An instruction usually has several encodings (widths, operand forms); a\nmnemonic counts as learned when any of its encodings is.\n");
    // group -> (encodings, encodings learned, mnemonic -> learned?)
    let mut groups: BTreeMap<&str, (usize, usize, BTreeMap<&str, bool>)> = BTreeMap::new();
    for (i, o) in official.iter().enumerate() {
        let g = groups.entry(o.group.as_str()).or_default();
        g.0 += 1;
        let hit = matched_officials.contains(&i);
        if hit {
            g.1 += 1;
        }
        *g.2.entry(o.mnemonic.as_str()).or_default() |= hit;
    }
    let _ = writeln!(text, "| group | encodings | learned | mnemonics | learned |\n|---|---|---|---|---|");
    for (g, (n, m, ms)) in &groups {
        let _ = writeln!(text, "| {} | {} | {} | {} | {} |", g, n, m, ms.len(), ms.values().filter(|v| **v).count());
    }
    for (g, (_, _, ms)) in &groups {
        if !focus.contains(g) {
            continue;
        }
        let missing: Vec<&str> = ms.iter().filter(|(_, v)| !**v).map(|(k, _)| *k).collect();
        if !missing.is_empty() {
            let _ = writeln!(text, "\nNot learned in {} ({} mnemonics): {}", g, missing.len(), missing.join(", "));
        }
    }
    let _ = writeln!(text);
    Ok(Card { text, problems })
}


#[cfg(test)]
mod tests {
    /// needs tools/get-isa-tables.sh to have run: cargo test -- --ignored
    #[test]
    #[ignore]
    fn every_learned_template_is_in_the_official_table() {
        for t in ["arm64", "riscv64", "wasm32"] {
            let card = super::scorecard(t).unwrap();
            assert_eq!(card.problems, 0, "{}:\n{}", t, card.text);
        }
    }
}
