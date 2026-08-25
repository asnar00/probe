---
name: explain
description: Explain probe in writing. Regenerates README.md (the whole project, for a casual reader) and appends to history.md (one phone-screen entry per commit or commit group). Use after landing a feature, or when asked to explain / document / write up the project or its latest change.
---

# /explain

Two documents, always written from the tree as it is right now — never
from memory of what it used to be or was planned to become.

Arguments: `/explain` does both. `/explain readme` or `/explain history`
does one.

## 0. Establish the ground truth first

Before writing a word:

- `git log --oneline -15`, and read the bodies of any commit not yet in
  history.md with `git show --stat`. A commit's story is in its body,
  not its title.
- Read `ssa.md` (the IR reference), `future-work.md`, and skim
  `src/*.rs` module headers so you describe what exists, not what a
  commit title promises.
- `cargo build -q` and run the suite (`probe test`, `test wasm`,
  `test riscv`, `test arm-qemu`) so every claim like "96/96 on all
  backends" is one you just watched be true. If it fails, say so in the
  doc rather than asserting it passes.

## 1. README.md — the project, for a casual reader

Audience: a curious programmer who has never seen the repo, reading for
five minutes. They should leave knowing *what probe is, why it's
interesting, and what's actually in the tree*.

Shape (keep it under ~120 lines):

1. One-paragraph pitch: the single idea (learn instruction encodings by
   probing a toolchain instead of transcribing manuals) and why that is
   worth doing.
2. The pipeline as a short diagram (source → SSA → emitter → bytes →
   verified), with the learned `targets/*.encodings.json` shown feeding
   the emitter.
3. "What's here": one bullet per real component, each pointing at its
   file(s). Only components that exist on `main` now. Check every path
   you name with `ls`.
4. How to run it: the actual commands from `src/main.rs`'s CLI and the
   suite, copied from the source, tried once.
5. Status / what's deliberately not here — one short paragraph, pulled
   from `future-work.md`.

Rewrite the file from scratch rather than patching the old one; stale
sentences survive patches. Preserve the existing tone: plain, concrete,
no marketing.

## 2. history.md — one short entry per commit

Audience: ash, scrolling on a phone. Each entry is a phone-screen of
text (60–100 words): what landed and why it matters, in the project's
own terms. Newest first.

- **Append, don't rewrite.** Entries for commits already in history.md
  stay as they are. Add entries only for commits newer than the top
  entry's hash (`git log <top-hash>..HEAD --reverse`).
- **Group when a burst is one piece of work**: several commits within an
  hour that build one feature (e.g. regalloc → sink/parallel moves →
  coalescing) get a single entry that names every hash, oldest → newest,
  with one clause per commit. A pure doc follow-up (`ssa.md: ...`) joins
  the commit it documents. Independent features never share an entry,
  even if adjacent.
- **Heading**: `### <Feature name> — \`hash\` [+ \`hash\`…] · YYYY-MM-DD`.
- **Body**: written from the commit body plus the diff, not the title
  alone. Name the file that owns the feature. Any number (suite count,
  bytes, speedup) comes from the commit message or from a run you just
  did — nothing remembered.
- If code is quoted at all, it is verbatim from the tree, labelled
  `path:lines`, and short. Usually none is needed here; that depth is
  what the commit body and `git show` are for.

## 3. Finish

- Re-read both docs once against `ls`, `git log`, and the suite output
  you captured. Any path, command, number, or claim you can't point at
  in the tree gets fixed or removed. Every hash in history.md must
  resolve (`git cat-file -e <hash>`).
- Don't commit unless asked; report what changed in one paragraph.
