#!/bin/sh
# Fetch and build Berkeley SoftFloat 3 + TestFloat 3, the IEEE-754 oracle
# `probe testfloat` runs against. The RISC-V specialization is the one
# whose NaN conventions (a single positive canonical NaN) match
# lib/float.ssa. Portable C: the x86_64 makefiles (the RISC-V ones pass
# -march) with the specialization overridden, and Apple's ar rather than
# binutils' when both are on the path.
set -e
cd "$(dirname "$0")"
[ -d berkeley-softfloat-3 ] || git clone -q https://github.com/ucb-bar/berkeley-softfloat-3.git
[ -d berkeley-testfloat-3 ] || git clone -q https://github.com/ucb-bar/berkeley-testfloat-3.git
AR=/usr/bin/ar; [ -x "$AR" ] || AR=ar
for d in berkeley-softfloat-3 berkeley-testfloat-3; do
    sed -i.bak "s|^MAKELIB = ar crs|MAKELIB = $AR crs|; s/-Werror-implicit-function-declaration//" $d/build/Linux-x86_64-GCC/Makefile
done
make -s -C berkeley-softfloat-3/build/Linux-x86_64-GCC -j8 SPECIALIZE_TYPE=RISCV
make -s -C berkeley-testfloat-3/build/Linux-x86_64-GCC -j8 SPECIALIZE_TYPE=RISCV testfloat_gen
ls -l berkeley-testfloat-3/build/Linux-x86_64-GCC/testfloat_gen
