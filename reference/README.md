# ARM64 references

Local copies:

- `a64-quickref.pdf` — AArch64 instruction set quick-reference sheet
  (from https://github.com/flynd/asmsheets, covers A64 up to ARMv8.6 incl.
  FP/SIMD). Mnemonic-level: syntax and semantics, not bit encodings.
- `armv8-isa-overview.pdf` — ARM's own "ARMv8 Instruction Set Overview"
  (PRD03-GENC-010197). Longer narrative overview of A64; still mnemonic-level.

Not downloaded (large), but important:

- **ARM Machine Readable Architecture (MRA)** — ARM publishes the full A64 ISA
  as XML: one file per instruction with the exact encoding diagram (bit fields,
  fixed bits, field names) plus ASL decode/execute semantics. This is the
  ground truth our prober's learned encodings can be validated against.
  - Download index: https://developer.arm.com/downloads/-/exploration-tools
  - Known-good direct link (v8.6, 2019-12):
    https://developer.arm.com/-/media/developer/products/architecture/armv8-a-architecture/2019-12/A64_ISA_xml_v86A-2019-12.tar.gz
  - Tools/notes for parsing it: https://github.com/alastairreid/mra_tools and
    https://alastairreid.github.io/dissecting-ARM-MRA/

Local oracles (no download needed): `llvm-mc -triple=arm64 -show-encoding`
assembles a single instruction and prints its bytes — much faster than a full
clang round trip for probing. `llvm-mc -disassemble` goes the other way.
