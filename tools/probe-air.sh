#!/bin/sh
# Probe Apple's GPU compiler: compile each tools/air/*.metal with the
# Metal Toolchain (xcodebuild -downloadComponent MetalToolchain), and
# keep what it emits — the IR text, the bitstream dump, and a metallib
# — as the reference src/bitcode.rs and src/emit_air.rs are written
# against. Apple's tools are used here to learn from, never at build
# time. Output: target/air/<name>.{ll,air,metallib,dump.txt}.
set -e
LLVM=/opt/homebrew/opt/llvm/bin
mkdir -p target/air
for f in tools/air/*.metal; do
    n=$(basename "$f" .metal)
    xcrun metal -S -emit-llvm -o "target/air/$n.ll" "$f"
    xcrun metal -c -o "target/air/$n.air" "$f"
    xcrun metallib -o "target/air/$n.metallib" "target/air/$n.air"
    "$LLVM/llvm-bcanalyzer" -dump "target/air/$n.air" > "target/air/$n.dump.txt" 2>&1 || true
    echo "$n: $(wc -l < target/air/$n.ll) lines of IR, $(wc -c < target/air/$n.air) bytes of bitcode"
done
