---
name: format
description: Scaffold a new number format as a probe library — lib/<name>.ssa with the type and operation generics, suite/<name>.ssa with directives, and the verification runs — following formats.md. Use when asked to add a number type, format, or family (a posit, a decimal, a complex, a duration...).
---

# /format <name> [description]

Follow `formats.md` step by step; it is the contract. Do not invent
compiler features: a format is a library.

1. Read `formats.md`, `ssa.md` (*Type declarations*, *Generic
   functions*, *Abstract numeric types*), and the closest existing
   library in `lib/` (`time.ssa` for an alias with units, `fixed.ssa` or
   `unit.ssa` for a small parametric type, `rational.ssa` or `float.ssa`
   for a full one). Copy their shape, not their bodies.
2. Write `lib/<name>.ssa`: a header comment saying what the values mean
   and what is exact, the `type`, then the operations as generics named
   after opcodes (`add`, `sub`, `mul`, `div`, `lt`..`ne`, `neg`, `abs`,
   `min`, `max`) plus `conv` from `i(W)` (and `float(E, M)` if fractions
   exist) so literals work, plus any named operations. Use a wide type
   for any intermediate that outgrows a word.
3. Write `suite/<name>.ssa` with `;!` directives whose expected values
   are computed independently (a short Python script in the transcript
   using `fractions`/`decimal`/plain integers is fine — say which). Cover
   edges: zero, signs, the largest and smallest values, whatever the
   format treats specially (NaR, saturation).
4. Run `cargo run -- test`, `test wasm`, `test riscv`, `test arm-qemu`,
   `--soft test`, and `cargo test`. Every backend must agree before the
   format is done.
5. Only if asked for a bare abstract name (`--<name>=`): the policy
   edits in `formats.md` §5, and a suite run under the new policy.
6. Add a `history.md` entry (see the `explain` skill's rules: verbatim
   code only) and report what landed in one paragraph. Don't commit
   unless asked.
