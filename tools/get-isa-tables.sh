#!/bin/sh
# Fetch the official instruction inventories `probe scorecard` checks the
# learned encodings against:
#   arm64   Arm's Machine Readable Architecture XML (A64 ISA, 2022-12
#           release; Arm's own download needs a browser, this is the
#           Internet Archive's mirror of the same tarball)
#   riscv64 riscv/riscv-opcodes, the encodings the RISC-V spec is
#           generated from
#   wasm32  wabt's opcode table, which follows the WebAssembly spec
set -e
cd "$(dirname "$0")"
[ -d ISA_A64_xml_A_profile-2022-12 ] || {
    curl -sL -o arm.tar.gz "https://archive.org/download/arm-xml-a-profile-2022-12/ISA_A64_xml_A_profile-2022-12.tar.gz"
    tar xzf arm.tar.gz 'ISA_A64_xml_A_profile-2022-12/*.xml'
    rm arm.tar.gz
}
[ -d riscv-opcodes ] || git clone -q --depth 1 https://github.com/riscv/riscv-opcodes.git
[ -d wabt ] || git clone -q --depth 1 https://github.com/WebAssembly/wabt.git
ls -d ISA_A64_xml_A_profile-2022-12 riscv-opcodes wabt/include/wabt/opcode.def
