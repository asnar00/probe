//! Software floating point, written in this compiler's own SSA — over the
//! `$fp` bitfield struct, so unpacking a double is field extraction and
//! the code reads like the IEEE layout. The `soften` pass rewrites float
//! SSA into integer SSA plus calls into this library, giving float
//! support to targets with no FPU at all.
//!
//! Scope: normals, zeros, infinities, and NaN (canonical quiet NaN out).
//! Subnormals flush to zero — documented tier-1 behavior. Rounding is
//! round-to-nearest-even via the standard guard/round/sticky scheme with
//! sticky jamming. f32 operations promote to f64, operate, and demote:
//! correctly rounded because 53 >= 2*24+2 (innocuous double rounding).
//!
//! The verification story: these functions run through the same JIT as
//! everything else, and the differential tests compare them bit-for-bit
//! against the host CPU's FPU on thousands of random values — hardware as
//! the oracle for software semantics.

use crate::ssa::{self, CastOp, FCond, Inst, Module, Type};

pub const RUNTIME: &str = r#"
type $fp = { sign: i1, exp: i11, frac: i52 }
type $fp32 = { sign: i1, exp: i8, frac: i23 }

fn @__fp_qnan() -> i64 {
    %q: i64 = iconst 0x7ff8000000000000
    ret %q
}

fn @__fp_isnan(%b: i64) -> i1 {
    %p: $fp = bitcast %b
    %e: i11 = extract %p, exp
    %f: i52 = extract %p, frac
    %emax: i11 = iconst 2047
    %isemax: i1 = icmp.eq %e, %emax
    %zf: i52 = iconst 0
    %fnz: i1 = icmp.ne %f, %zf
    %x: i64 = zext %isemax
    %y: i64 = zext %fnz
    %z: i64 = and %x, %y
    %r: i1 = trunc %z
    ret %r
}

fn @__fp_zero(%s: i1) -> i64 {
    %e: i11 = iconst 0
    %f: i52 = iconst 0
    %p: $fp = pack %s, %e, %f
    %r: i64 = bitcast %p
    ret %r
}

fn @__fp_inf(%s: i1) -> i64 {
    %e: i11 = iconst 2047
    %f: i52 = iconst 0
    %p: $fp = pack %s, %e, %f
    %r: i64 = bitcast %p
    ret %r
}

fn @__fp_pack(%s: i1, %e: i64, %m: i64) -> i64 {
    %e11: i11 = trunc %e
    %m52: i52 = trunc %m
    %p: $fp = pack %s, %e11, %m52
    %r: i64 = bitcast %p
    ret %r
}

; m56 holds 1.frac plus guard/round/sticky in [2^55, 2^56); round to
; nearest even, overflowing to infinity, flushing e <= 0 to zero
fn @__fp_round(%s: i1, %e: i64, %m56: i64) -> i64 {
    %one: i64 = iconst 1
    %three: i64 = iconst 3
    %zero: i64 = iconst 0
    %m: i64 = lshr %m56, %three
    %gsh: i64 = iconst 2
    %g0: i64 = lshr %m56, %gsh
    %g: i64 = and %g0, %one
    %rs: i64 = and %m56, %three
    %lsb: i64 = and %m, %one
    %any0: i64 = or %rs, %lsb
    %any: i1 = icmp.ne %any0, %zero
    %anyi: i64 = zext %any
    %up: i64 = and %g, %anyi
    %m2: i64 = iadd %m, %up
    %top: i64 = iconst 0x20000000000000
    %ovf: i1 = icmp.eq %m2, %top
    %ef: i64, %mf: i64 = if %ovf {
        %m3: i64 = lshr %m2, %one
        %e2: i64 = iadd %e, %one
        yield %e2, %m3
    } else {
        yield %e, %m2
    }
    %emax: i64 = iconst 2047
    %oo: i1 = icmp.sge %ef, %emax
    if %oo {
        %inf: i64 = call @__fp_inf(%s)
        ret %inf
    }
    %uu: i1 = icmp.sle %ef, %zero
    if %uu {
        %z: i64 = call @__fp_zero(%s)
        ret %z
    }
    %r: i64 = call @__fp_pack(%s, %ef, %mf)
    ret %r
}

fn @__f64_add(%a: i64, %b: i64) -> i64 {
    %an: i1 = call @__fp_isnan(%a)
    if %an {
        %q1: i64 = call @__fp_qnan()
        ret %q1
    }
    %bn: i1 = call @__fp_isnan(%b)
    if %bn {
        %q2: i64 = call @__fp_qnan()
        ret %q2
    }
    %pa: $fp = bitcast %a
    %pb: $fp = bitcast %b
    %sa: i1 = extract %pa, sign
    %sb: i1 = extract %pb, sign
    %ea0: i11 = extract %pa, exp
    %eb0: i11 = extract %pb, exp
    %fa0: i52 = extract %pa, frac
    %fb0: i52 = extract %pb, frac
    %ea: i64 = zext %ea0
    %eb: i64 = zext %eb0
    %fa: i64 = zext %fa0
    %fb: i64 = zext %fb0
    %emax: i64 = iconst 2047
    %zero: i64 = iconst 0
    %one: i64 = iconst 1
    %ainf: i1 = icmp.eq %ea, %emax
    if %ainf {
        %binf: i1 = icmp.eq %eb, %emax
        if %binf {
            %same: i1 = icmp.eq %a, %b
            if %same {
                ret %a
            }
            %q3: i64 = call @__fp_qnan()
            ret %q3
        }
        ret %a
    }
    %binf2: i1 = icmp.eq %eb, %emax
    if %binf2 {
        ret %b
    }
    %az: i1 = icmp.eq %ea, %zero
    %bz: i1 = icmp.eq %eb, %zero
    %azi: i64 = zext %az
    %bzi: i64 = zext %bz
    %bothz0: i64 = and %azi, %bzi
    %bothz: i1 = trunc %bothz0
    if %bothz {
        %sai: i64 = zext %sa
        %sbi: i64 = zext %sb
        %sb0: i64 = and %sai, %sbi
        %sr: i1 = trunc %sb0
        %z1: i64 = call @__fp_zero(%sr)
        ret %z1
    }
    if %az {
        ret %b
    }
    if %bz {
        ret %a
    }
    %hid: i64 = iconst 0x10000000000000
    %Ma: i64 = or %fa, %hid
    %Mb: i64 = or %fb, %hid
    ; order operands so the larger magnitude comes first
    %lte: i1 = icmp.slt %ea, %eb
    %eqe: i1 = icmp.eq %ea, %eb
    %ltm: i1 = icmp.ult %Ma, %Mb
    %eqei: i64 = zext %eqe
    %ltmi: i64 = zext %ltm
    %t0: i64 = and %eqei, %ltmi
    %ltei: i64 = zext %lte
    %sw0: i64 = or %ltei, %t0
    %swap: i1 = trunc %sw0
    %sx: i1, %sy: i1, %e1: i64, %M1: i64, %e2: i64, %M2: i64 = if %swap {
        yield %sb, %sa, %eb, %Mb, %ea, %Ma
    } else {
        yield %sa, %sb, %ea, %Ma, %eb, %Mb
    }
    %d: i64 = isub %e1, %e2
    %three: i64 = iconst 3
    %M13: i64 = shl %M1, %three
    %M23: i64 = shl %M2, %three
    %c60: i64 = iconst 60
    %far: i1 = icmp.sgt %d, %c60
    %sh: i64, %st: i64 = if %far {
        yield %zero, %one
    } else {
        %sh0: i64 = lshr %M23, %d
        %msk0: i64 = shl %one, %d
        %msk: i64 = isub %msk0, %one
        %lost: i64 = and %M23, %msk
        %lnz: i1 = icmp.ne %lost, %zero
        %sti: i64 = zext %lnz
        yield %sh0, %sti
    }
    %sxi: i64 = zext %sx
    %syi: i64 = zext %sy
    %sdif: i64 = xor %sxi, %syi
    %ssame: i1 = icmp.eq %sdif, %zero
    %m56: i64, %ef: i64 = if %ssame {
        %shj: i64 = or %sh, %st
        %sum: i64 = iadd %M13, %shj
        %lim: i64 = iconst 0x100000000000000
        %c: i1 = icmp.uge %sum, %lim
        %mm: i64, %ee: i64 = if %c {
            %lo: i64 = and %sum, %one
            %s1: i64 = lshr %sum, %one
            %s2: i64 = or %s1, %lo
            %ep: i64 = iadd %e1, %one
            yield %s2, %ep
        } else {
            yield %sum, %e1
        }
        yield %mm, %ee
    } else {
        %d0: i64 = isub %M13, %sh
        %d1: i64 = isub %d0, %st
        %dj: i64 = or %d1, %st
        %cz: i1 = icmp.eq %dj, %zero
        if %cz {
            %pos: i1 = iconst 0
            %z2: i64 = call @__fp_zero(%pos)
            ret %z2
        }
        %top55: i64 = iconst 0x80000000000000
        %mn: i64, %en: i64 = loop(%mc: i64 = %dj, %ec: i64 = %e1) {
            %ok: i1 = icmp.uge %mc, %top55
            if %ok {
                break %mc, %ec
            }
            %mc2: i64 = shl %mc, %one
            %ec2: i64 = isub %ec, %one
            continue %mc2, %ec2
        }
        yield %mn, %en
    }
    %r: i64 = call @__fp_round(%sx, %ef, %m56)
    ret %r
}

fn @__f64_sub(%a: i64, %b: i64) -> i64 {
    %sgn: i64 = iconst 0x8000000000000000
    %nb: i64 = xor %b, %sgn
    %r: i64 = call @__f64_add(%a, %nb)
    ret %r
}

fn @__f64_mul(%a: i64, %b: i64) -> i64 {
    %an: i1 = call @__fp_isnan(%a)
    if %an {
        %q1: i64 = call @__fp_qnan()
        ret %q1
    }
    %bn: i1 = call @__fp_isnan(%b)
    if %bn {
        %q2: i64 = call @__fp_qnan()
        ret %q2
    }
    %pa: $fp = bitcast %a
    %pb: $fp = bitcast %b
    %sa: i1 = extract %pa, sign
    %sb: i1 = extract %pb, sign
    %ea0: i11 = extract %pa, exp
    %eb0: i11 = extract %pb, exp
    %fa0: i52 = extract %pa, frac
    %fb0: i52 = extract %pb, frac
    %ea: i64 = zext %ea0
    %eb: i64 = zext %eb0
    %fa: i64 = zext %fa0
    %fb: i64 = zext %fb0
    %sai: i64 = zext %sa
    %sbi: i64 = zext %sb
    %sx0: i64 = xor %sai, %sbi
    %sx: i1 = trunc %sx0
    %emax: i64 = iconst 2047
    %zero: i64 = iconst 0
    %az: i1 = icmp.eq %ea, %zero
    %bz: i1 = icmp.eq %eb, %zero
    %ainf: i1 = icmp.eq %ea, %emax
    if %ainf {
        if %bz {
            %q3: i64 = call @__fp_qnan()
            ret %q3
        }
        %i1_: i64 = call @__fp_inf(%sx)
        ret %i1_
    }
    %binf: i1 = icmp.eq %eb, %emax
    if %binf {
        if %az {
            %q4: i64 = call @__fp_qnan()
            ret %q4
        }
        %i2_: i64 = call @__fp_inf(%sx)
        ret %i2_
    }
    if %az {
        %z1: i64 = call @__fp_zero(%sx)
        ret %z1
    }
    if %bz {
        %z2: i64 = call @__fp_zero(%sx)
        ret %z2
    }
    %hid: i64 = iconst 0x10000000000000
    %Ma: i64 = or %fa, %hid
    %Mb: i64 = or %fb, %hid
    %bias: i64 = iconst 1023
    %es: i64 = iadd %ea, %eb
    %e: i64 = isub %es, %bias
    ; 53x53 -> 106-bit product from 32-bit partials
    %m32: i64 = iconst 0xffffffff
    %c32: i64 = iconst 32
    %a0: i64 = and %Ma, %m32
    %a1: i64 = lshr %Ma, %c32
    %b0: i64 = and %Mb, %m32
    %b1: i64 = lshr %Mb, %c32
    %p00: i64 = imul %a0, %b0
    %p01: i64 = imul %a0, %b1
    %p10: i64 = imul %a1, %b0
    %p11: i64 = imul %a1, %b1
    %p00h: i64 = lshr %p00, %c32
    %p01l: i64 = and %p01, %m32
    %p10l: i64 = and %p10, %m32
    %mid0: i64 = iadd %p00h, %p01l
    %mid: i64 = iadd %mid0, %p10l
    %midl: i64 = and %mid, %m32
    %los: i64 = shl %midl, %c32
    %p00l: i64 = and %p00, %m32
    %lo: i64 = or %los, %p00l
    %p01h: i64 = lshr %p01, %c32
    %p10h: i64 = lshr %p10, %c32
    %midh: i64 = lshr %mid, %c32
    %hi0: i64 = iadd %p11, %p01h
    %hi1: i64 = iadd %hi0, %p10h
    %hi: i64 = iadd %hi1, %midh
    ; take 56 bits from the top of the 106-bit product
    %c49: i64 = iconst 49
    %c15: i64 = iconst 15
    %th: i64 = shl %hi, %c15
    %tl: i64 = lshr %lo, %c49
    %t0_: i64 = or %th, %tl
    %lm0: i64 = iconst 0x1ffffffffffff
    %lost: i64 = and %lo, %lm0
    %lnz: i1 = icmp.ne %lost, %zero
    %sti: i64 = zext %lnz
    %t: i64 = or %t0_, %sti
    %lim: i64 = iconst 0x100000000000000
    %c: i1 = icmp.uge %t, %lim
    %one: i64 = iconst 1
    %m56: i64, %ef: i64 = if %c {
        %lo1: i64 = and %t, %one
        %t1: i64 = lshr %t, %one
        %t2: i64 = or %t1, %lo1
        %e2: i64 = iadd %e, %one
        yield %t2, %e2
    } else {
        yield %t, %e
    }
    %r: i64 = call @__fp_round(%sx, %ef, %m56)
    ret %r
}

fn @__f64_div(%a: i64, %b: i64) -> i64 {
    %an: i1 = call @__fp_isnan(%a)
    if %an {
        %q1: i64 = call @__fp_qnan()
        ret %q1
    }
    %bn: i1 = call @__fp_isnan(%b)
    if %bn {
        %q2: i64 = call @__fp_qnan()
        ret %q2
    }
    %pa: $fp = bitcast %a
    %pb: $fp = bitcast %b
    %sa: i1 = extract %pa, sign
    %sb: i1 = extract %pb, sign
    %ea0: i11 = extract %pa, exp
    %eb0: i11 = extract %pb, exp
    %fa0: i52 = extract %pa, frac
    %fb0: i52 = extract %pb, frac
    %ea: i64 = zext %ea0
    %eb: i64 = zext %eb0
    %fa: i64 = zext %fa0
    %fb: i64 = zext %fb0
    %sai: i64 = zext %sa
    %sbi: i64 = zext %sb
    %sx0: i64 = xor %sai, %sbi
    %sx: i1 = trunc %sx0
    %emax: i64 = iconst 2047
    %zero: i64 = iconst 0
    %az: i1 = icmp.eq %ea, %zero
    %bz: i1 = icmp.eq %eb, %zero
    %ainf: i1 = icmp.eq %ea, %emax
    %binf: i1 = icmp.eq %eb, %emax
    if %ainf {
        if %binf {
            %q3: i64 = call @__fp_qnan()
            ret %q3
        }
        %i1_: i64 = call @__fp_inf(%sx)
        ret %i1_
    }
    if %binf {
        %z1: i64 = call @__fp_zero(%sx)
        ret %z1
    }
    if %bz {
        if %az {
            %q4: i64 = call @__fp_qnan()
            ret %q4
        }
        %i2_: i64 = call @__fp_inf(%sx)
        ret %i2_
    }
    if %az {
        %z2: i64 = call @__fp_zero(%sx)
        ret %z2
    }
    %hid: i64 = iconst 0x10000000000000
    %Ma: i64 = or %fa, %hid
    %Mb: i64 = or %fb, %hid
    %bias: i64 = iconst 1023
    %ed: i64 = isub %ea, %eb
    %e0: i64 = iadd %ed, %bias
    %one: i64 = iconst 1
    %small: i1 = icmp.ult %Ma, %Mb
    %rem0: i64, %e: i64 = if %small {
        %r2: i64 = shl %Ma, %one
        %e1: i64 = isub %e0, %one
        yield %r2, %e1
    } else {
        yield %Ma, %e0
    }
    ; 55 quotient bits by restoring division
    %c55: i64 = iconst 55
    %q: i64, %rem: i64 = loop(%i: i64 = %zero, %qq: i64 = %zero, %rr: i64 = %rem0) {
        %done: i1 = icmp.sge %i, %c55
        if %done {
            break %qq, %rr
        }
        %q1_: i64 = shl %qq, %one
        %ge: i1 = icmp.uge %rr, %Mb
        %q2_: i64, %r2_: i64 = if %ge {
            %rs: i64 = isub %rr, %Mb
            %qo: i64 = or %q1_, %one
            yield %qo, %rs
        } else {
            yield %q1_, %rr
        }
        %r3_: i64 = shl %r2_, %one
        %i2: i64 = iadd %i, %one
        continue %i2, %q2_, %r3_
    }
    %rnz: i1 = icmp.ne %rem, %zero
    %sti: i64 = zext %rnz
    %m0: i64 = shl %q, %one
    %m56: i64 = or %m0, %sti
    %r: i64 = call @__fp_round(%sx, %e, %m56)
    ret %r
}

; ordered comparisons; magnitudes flush subnormals like the arithmetic
fn @__f64_eq(%a: i64, %b: i64) -> i1 {
    %f: i1 = iconst 0
    %t: i1 = iconst 1
    %an: i1 = call @__fp_isnan(%a)
    if %an {
        ret %f
    }
    %bn: i1 = call @__fp_isnan(%b)
    if %bn {
        ret %f
    }
    %sgn: i64 = iconst 0x8000000000000000
    %nsgn: i64 = iconst 0x7fffffffffffffff
    %ma: i64 = and %a, %nsgn
    %mb: i64 = and %b, %nsgn
    %zero: i64 = iconst 0
    %az: i1 = icmp.eq %ma, %zero
    %bz: i1 = icmp.eq %mb, %zero
    %azi: i64 = zext %az
    %bzi: i64 = zext %bz
    %both0: i64 = and %azi, %bzi
    %both: i1 = trunc %both0
    if %both {
        ret %t
    }
    %r: i1 = icmp.eq %a, %b
    ret %r
}

fn @__f64_ne(%a: i64, %b: i64) -> i1 {
    %t: i1 = iconst 1
    %an: i1 = call @__fp_isnan(%a)
    if %an {
        ret %t
    }
    %bn: i1 = call @__fp_isnan(%b)
    if %bn {
        ret %t
    }
    %e: i1 = call @__f64_eq(%a, %b)
    %ei: i64 = zext %e
    %one: i64 = iconst 1
    %ni: i64 = xor %ei, %one
    %r: i1 = trunc %ni
    ret %r
}

fn @__f64_lt(%a: i64, %b: i64) -> i1 {
    %f: i1 = iconst 0
    %an: i1 = call @__fp_isnan(%a)
    if %an {
        ret %f
    }
    %bn: i1 = call @__fp_isnan(%b)
    if %bn {
        ret %f
    }
    %nsgn: i64 = iconst 0x7fffffffffffffff
    %ma: i64 = and %a, %nsgn
    %mb: i64 = and %b, %nsgn
    %zero: i64 = iconst 0
    %az: i1 = icmp.eq %ma, %zero
    %bz: i1 = icmp.eq %mb, %zero
    %azi: i64 = zext %az
    %bzi: i64 = zext %bz
    %both0: i64 = and %azi, %bzi
    %both: i1 = trunc %both0
    if %both {
        ret %f
    }
    %c63: i64 = iconst 63
    %sa: i64 = lshr %a, %c63
    %sb: i64 = lshr %b, %c63
    %sdiff: i1 = icmp.ne %sa, %sb
    if %sdiff {
        %r1: i1 = trunc %sa
        ret %r1
    }
    %sneg: i1 = trunc %sa
    if %sneg {
        %r2: i1 = icmp.ugt %ma, %mb
        ret %r2
    }
    %r3: i1 = icmp.ult %ma, %mb
    ret %r3
}

fn @__f64_le(%a: i64, %b: i64) -> i1 {
    %f: i1 = iconst 0
    %an: i1 = call @__fp_isnan(%a)
    if %an {
        ret %f
    }
    %bn: i1 = call @__fp_isnan(%b)
    if %bn {
        ret %f
    }
    %lt: i1 = call @__f64_lt(%b, %a)
    %li: i64 = zext %lt
    %one: i64 = iconst 1
    %ni: i64 = xor %li, %one
    %r: i1 = trunc %ni
    ret %r
}

fn @__f64_from_i64(%n: i64) -> i64 {
    %zero: i64 = iconst 0
    %isz: i1 = icmp.eq %n, %zero
    if %isz {
        ret %zero
    }
    %neg: i1 = icmp.slt %n, %zero
    %mag: i64 = if %neg {
        %m0: i64 = isub %zero, %n
        yield %m0
    } else {
        yield %n
    }
    ; find the highest set bit
    %c63: i64 = iconst 63
    %one: i64 = iconst 1
    %hb: i64 = loop(%i: i64 = %c63) {
        %sh: i64 = lshr %mag, %i
        %bit: i64 = and %sh, %one
        %set: i1 = icmp.ne %bit, %zero
        if %set {
            break %i
        }
        %i2: i64 = isub %i, %one
        continue %i2
    }
    %bias: i64 = iconst 1023
    %e: i64 = iadd %bias, %hb
    %c52: i64 = iconst 52
    %fits: i1 = icmp.sle %hb, %c52
    if %fits {
        %shl_: i64 = isub %c52, %hb
        %m: i64 = shl %mag, %shl_
        %r1: i64 = call @__fp_pack(%neg, %e, %m)
        ret %r1
    }
    %sh2: i64 = isub %hb, %c52
    %keep: i64 = lshr %mag, %sh2
    %gsh: i64 = isub %sh2, %one
    %g0: i64 = lshr %mag, %gsh
    %g: i64 = and %g0, %one
    %rm0: i64 = shl %one, %gsh
    %rm: i64 = isub %rm0, %one
    %rest: i64 = and %mag, %rm
    %rnz: i1 = icmp.ne %rest, %zero
    %sti: i64 = zext %rnz
    %lsb: i64 = and %keep, %one
    %any0: i64 = or %sti, %lsb
    %anynz: i1 = icmp.ne %any0, %zero
    %anyi: i64 = zext %anynz
    %up: i64 = and %g, %anyi
    %m2: i64 = iadd %keep, %up
    %top: i64 = iconst 0x20000000000000
    %ovf: i1 = icmp.eq %m2, %top
    %ef: i64, %mf: i64 = if %ovf {
        %m3: i64 = lshr %m2, %one
        %e2: i64 = iadd %e, %one
        yield %e2, %m3
    } else {
        yield %e, %m2
    }
    %r2: i64 = call @__fp_pack(%neg, %ef, %mf)
    ret %r2
}

fn @__f64_from_u64(%n: i64) -> i64 {
    %zero: i64 = iconst 0
    %neg: i1 = icmp.slt %n, %zero
    if %neg {
        ; top bit set: halve (losing nothing to rounding only if even;
        ; fold the lost bit into the value via from_i64 of n/2*2? standard:
        ; convert (n >> 1 | n & 1) and double
        %one: i64 = iconst 1
        %h0: i64 = lshr %n, %one
        %lo: i64 = and %n, %one
        %h: i64 = or %h0, %lo
        %d: i64 = call @__f64_from_i64(%h)
        %two: i64 = iconst 0x4000000000000000
        %r0: i64 = call @__f64_mul(%d, %two)
        ret %r0
    }
    %r: i64 = call @__f64_from_i64(%n)
    ret %r
}

fn @__f64_to_i64(%a: i64) -> i64 {
    %zero: i64 = iconst 0
    %an: i1 = call @__fp_isnan(%a)
    if %an {
        ret %zero
    }
    %p: $fp = bitcast %a
    %s: i1 = extract %p, sign
    %e0: i11 = extract %p, exp
    %f0: i52 = extract %p, frac
    %e: i64 = zext %e0
    %f: i64 = zext %f0
    %bias: i64 = iconst 1023
    %lt1: i1 = icmp.slt %e, %bias
    if %lt1 {
        ret %zero
    }
    %sh: i64 = isub %e, %bias
    %c62: i64 = iconst 62
    %huge: i1 = icmp.sgt %sh, %c62
    if %huge {
        if %s {
            %min: i64 = iconst 0x8000000000000000
            ret %min
        }
        %max: i64 = iconst 0x7fffffffffffffff
        ret %max
    }
    %hid: i64 = iconst 0x10000000000000
    %M: i64 = or %f, %hid
    %c52: i64 = iconst 52
    %left: i1 = icmp.sgt %sh, %c52
    %r0: i64 = if %left {
        %k1: i64 = isub %sh, %c52
        %v1: i64 = shl %M, %k1
        yield %v1
    } else {
        %k2: i64 = isub %c52, %sh
        %v2: i64 = lshr %M, %k2
        yield %v2
    }
    if %s {
        %n: i64 = isub %zero, %r0
        ret %n
    }
    ret %r0
}

fn @__f64_to_u64(%a: i64) -> i64 {
    %zero: i64 = iconst 0
    %c63: i64 = iconst 63
    %sa: i64 = lshr %a, %c63
    %neg: i1 = trunc %sa
    if %neg {
        ret %zero
    }
    %r: i64 = call @__f64_to_i64(%a)
    ret %r
}

fn @__f64_from_i32(%n: i32) -> i64 {
    %w: i64 = sext %n
    %r: i64 = call @__f64_from_i64(%w)
    ret %r
}

fn @__f64_from_u32(%n: i32) -> i64 {
    %w: i64 = zext %n
    %r: i64 = call @__f64_from_i64(%w)
    ret %r
}

fn @__f64_to_i32(%a: i64) -> i32 {
    %w: i64 = call @__f64_to_i64(%a)
    %r: i32 = trunc %w
    ret %r
}

fn @__f64_to_u32(%a: i64) -> i32 {
    %w: i64 = call @__f64_to_u64(%a)
    %r: i32 = trunc %w
    ret %r
}

fn @__f64_from_f32(%b: i32) -> i64 {
    %p: $fp32 = bitcast %b
    %s: i1 = extract %p, sign
    %e0: i8 = extract %p, exp
    %f0: i23 = extract %p, frac
    %e8: i64 = zext %e0
    %f: i64 = zext %f0
    %c255: i64 = iconst 255
    %zero: i64 = iconst 0
    %isinfnan: i1 = icmp.eq %e8, %c255
    if %isinfnan {
        %fz: i1 = icmp.eq %f, %zero
        if %fz {
            %inf: i64 = call @__fp_inf(%s)
            ret %inf
        }
        %q: i64 = call @__fp_qnan()
        ret %q
    }
    %isz: i1 = icmp.eq %e8, %zero
    if %isz {
        %z: i64 = call @__fp_zero(%s)
        ret %z
    }
    %c896: i64 = iconst 896
    %e: i64 = iadd %e8, %c896
    %c29: i64 = iconst 29
    %m: i64 = shl %f, %c29
    %r: i64 = call @__fp_pack(%s, %e, %m)
    ret %r
}

fn @__f32_from_f64(%a: i64) -> i32 {
    %an: i1 = call @__fp_isnan(%a)
    if %an {
        %q: i32 = iconst 0x7fc00000
        ret %q
    }
    %p: $fp = bitcast %a
    %s: i1 = extract %p, sign
    %e0: i11 = extract %p, exp
    %f0: i52 = extract %p, frac
    %e: i64 = zext %e0
    %f: i64 = zext %f0
    %si: i64 = zext %s
    %c31: i64 = iconst 31
    %stop: i64 = shl %si, %c31
    %sbits: i32 = trunc %stop
    %emax: i64 = iconst 2047
    %zero: i64 = iconst 0
    %isinf: i1 = icmp.eq %e, %emax
    if %isinf {
        %infc: i32 = iconst 0x7f800000
        %r0: i32 = or %sbits, %infc
        ret %r0
    }
    %isz: i1 = icmp.eq %e, %zero
    if %isz {
        ret %sbits
    }
    %c896: i64 = iconst 896
    %e32: i64 = isub %e, %c896
    %c255: i64 = iconst 255
    %tooBig: i1 = icmp.sge %e32, %c255
    if %tooBig {
        %infc2: i32 = iconst 0x7f800000
        %r1: i32 = or %sbits, %infc2
        ret %r1
    }
    %tooSmall: i1 = icmp.sle %e32, %zero
    if %tooSmall {
        ret %sbits
    }
    %c29: i64 = iconst 29
    %keep: i64 = lshr %f, %c29
    %c28: i64 = iconst 28
    %g0: i64 = lshr %f, %c28
    %one: i64 = iconst 1
    %g: i64 = and %g0, %one
    %rm: i64 = iconst 0xfffffff
    %rest: i64 = and %f, %rm
    %rnz: i1 = icmp.ne %rest, %zero
    %sti: i64 = zext %rnz
    %lsb: i64 = and %keep, %one
    %any0: i64 = or %sti, %lsb
    %anynz: i1 = icmp.ne %any0, %zero
    %anyi: i64 = zext %anynz
    %up: i64 = and %g, %anyi
    %m2: i64 = iadd %keep, %up
    %m23top: i64 = iconst 0x800000
    %ovf: i1 = icmp.eq %m2, %m23top
    %ef: i64, %mf: i64 = if %ovf {
        %e2: i64 = iadd %e32, %one
        yield %e2, %zero
    } else {
        yield %e32, %m2
    }
    %again: i1 = icmp.sge %ef, %c255
    if %again {
        %infc3: i32 = iconst 0x7f800000
        %r2: i32 = or %sbits, %infc3
        ret %r2
    }
    %e8: i8 = trunc %ef
    %m23: i23 = trunc %mf
    %sp: $fp32 = pack %s, %e8, %m23
    %r: i32 = bitcast %sp
    ret %r
}

; f32 arithmetic: promote, operate in f64, demote — correctly rounded
; because 53 >= 2*24 + 2 (innocuous double rounding)
fn @__f32_add(%a: i32, %b: i32) -> i32 {
    %wa: i64 = call @__f64_from_f32(%a)
    %wb: i64 = call @__f64_from_f32(%b)
    %w: i64 = call @__f64_add(%wa, %wb)
    %r: i32 = call @__f32_from_f64(%w)
    ret %r
}

fn @__f32_sub(%a: i32, %b: i32) -> i32 {
    %wa: i64 = call @__f64_from_f32(%a)
    %wb: i64 = call @__f64_from_f32(%b)
    %w: i64 = call @__f64_sub(%wa, %wb)
    %r: i32 = call @__f32_from_f64(%w)
    ret %r
}

fn @__f32_mul(%a: i32, %b: i32) -> i32 {
    %wa: i64 = call @__f64_from_f32(%a)
    %wb: i64 = call @__f64_from_f32(%b)
    %w: i64 = call @__f64_mul(%wa, %wb)
    %r: i32 = call @__f32_from_f64(%w)
    ret %r
}

fn @__f32_div(%a: i32, %b: i32) -> i32 {
    %wa: i64 = call @__f64_from_f32(%a)
    %wb: i64 = call @__f64_from_f32(%b)
    %w: i64 = call @__f64_div(%wa, %wb)
    %r: i32 = call @__f32_from_f64(%w)
    ret %r
}

fn @__f32_eq(%a: i32, %b: i32) -> i1 {
    %wa: i64 = call @__f64_from_f32(%a)
    %wb: i64 = call @__f64_from_f32(%b)
    %r: i1 = call @__f64_eq(%wa, %wb)
    ret %r
}

fn @__f32_ne(%a: i32, %b: i32) -> i1 {
    %wa: i64 = call @__f64_from_f32(%a)
    %wb: i64 = call @__f64_from_f32(%b)
    %r: i1 = call @__f64_ne(%wa, %wb)
    ret %r
}

fn @__f32_lt(%a: i32, %b: i32) -> i1 {
    %wa: i64 = call @__f64_from_f32(%a)
    %wb: i64 = call @__f64_from_f32(%b)
    %r: i1 = call @__f64_lt(%wa, %wb)
    ret %r
}

fn @__f32_le(%a: i32, %b: i32) -> i1 {
    %wa: i64 = call @__f64_from_f32(%a)
    %wb: i64 = call @__f64_from_f32(%b)
    %r: i1 = call @__f64_le(%wa, %wb)
    ret %r
}

fn @__f32_from_i64(%n: i64) -> i32 {
    %w: i64 = call @__f64_from_i64(%n)
    %r: i32 = call @__f32_from_f64(%w)
    ret %r
}

fn @__f32_from_u64(%n: i64) -> i32 {
    %w: i64 = call @__f64_from_u64(%n)
    %r: i32 = call @__f32_from_f64(%w)
    ret %r
}

fn @__f32_from_i32(%n: i32) -> i32 {
    %w: i64 = call @__f64_from_i32(%n)
    %r: i32 = call @__f32_from_f64(%w)
    ret %r
}

fn @__f32_from_u32(%n: i32) -> i32 {
    %w: i64 = call @__f64_from_u32(%n)
    %r: i32 = call @__f32_from_f64(%w)
    ret %r
}

fn @__f32_to_i64(%a: i32) -> i64 {
    %w: i64 = call @__f64_from_f32(%a)
    %r: i64 = call @__f64_to_i64(%w)
    ret %r
}

fn @__f32_to_u64(%a: i32) -> i64 {
    %w: i64 = call @__f64_from_f32(%a)
    %r: i64 = call @__f64_to_u64(%w)
    ret %r
}

fn @__f32_to_i32(%a: i32) -> i32 {
    %w: i64 = call @__f32_to_i64(%a)
    %r: i32 = trunc %w
    ret %r
}

fn @__f32_to_u32(%a: i32) -> i32 {
    %w: i64 = call @__f32_to_u64(%a)
    %r: i32 = trunc %w
    ret %r
}
"#;

// ---------------------------------------------------------------------------
// The soften pass: float SSA -> integer SSA + runtime calls.
//
// Because types live on variables, the shape is familiar: rewrite float
// instructions into calls (or substitutions, for bitcasts that become
// identities), then retype f64 -> i64 and f32 -> i32. The runtime's
// functions are appended to the module afterwards; they are already
// integer/struct SSA and go through the normal lowering.

pub fn soften(module: &mut Module) -> Result<(), String> {
    if module.funcs.iter().any(|f| f.name == "__f64_add") {
        return Ok(()); // already softened / library present
    }
    let uses_floats = module.funcs.iter().any(|f| {
        f.values
            .iter()
            .any(|v| matches!(v.ty, Type::F32 | Type::F64))
    });
    for func in &mut module.funcs {
        soften_function(func);
    }
    if !uses_floats {
        return Ok(());
    }
    let lib = ssa::parse(RUNTIME).map_err(|e| format!("softfloat runtime: {}", e))?;
    module.funcs.extend(lib.funcs);
    Ok(())
}

fn soften_function(func: &mut ssa::Function) {
    use std::collections::HashMap;
    let mut subst: HashMap<ssa::ValueId, ssa::ValueId> = HashMap::new();
    let f64ty = |t: Type| t == Type::F64;
    for b in 0..func.blocks.len() {
        let insts = std::mem::take(&mut func.blocks[b].insts);
        let mut out = Vec::with_capacity(insts.len());
        for inst in insts {
            match inst {
                Inst::FConst { dst, bits } => {
                    let imm = if func.ty(dst) == Type::F32 {
                        (f64::from_bits(bits) as f32).to_bits() as i64
                    } else {
                        bits as i64
                    };
                    out.push(Inst::IConst { dst, imm });
                }
                Inst::Bin { op, dst, lhs, rhs } if op.is_float() => {
                    let wide = f64ty(func.ty(dst));
                    let base = match op {
                        ssa::BinOp::FAdd => "add",
                        ssa::BinOp::FSub => "sub",
                        ssa::BinOp::FMul => "mul",
                        ssa::BinOp::FDiv => "div",
                        _ => unreachable!(),
                    };
                    out.push(Inst::Call {
                        dsts: vec![dst],
                        callee: format!("__f{}_{}", if wide { 64 } else { 32 }, base),
                        args: vec![lhs, rhs],
                    });
                }
                Inst::FCmp {
                    cond,
                    dst,
                    lhs,
                    rhs,
                } => {
                    let wide = f64ty(func.ty(lhs));
                    // gt/ge are lt/le with the operands swapped
                    let (base, a, b2) = match cond {
                        FCond::Oeq => ("eq", lhs, rhs),
                        FCond::Une => ("ne", lhs, rhs),
                        FCond::Olt => ("lt", lhs, rhs),
                        FCond::Ole => ("le", lhs, rhs),
                        FCond::Ogt => ("lt", rhs, lhs),
                        FCond::Oge => ("le", rhs, lhs),
                    };
                    out.push(Inst::Call {
                        dsts: vec![dst],
                        callee: format!("__f{}_{}", if wide { 64 } else { 32 }, base),
                        args: vec![a, b2],
                    });
                }
                Inst::Cast { op, dst, src } => {
                    let from = func.ty(src);
                    let to = func.ty(dst);
                    let call = |name: String, out: &mut Vec<Inst>| {
                        out.push(Inst::Call {
                            dsts: vec![dst],
                            callee: name,
                            args: vec![src],
                        });
                    };
                    match op {
                        CastOp::Sitofp | CastOp::Uitofp => {
                            let u = if op == CastOp::Uitofp { "u" } else { "i" };
                            let fw = if f64ty(to) { 64 } else { 32 };
                            let iw = if from == Type::I64 { 64 } else { 32 };
                            call(format!("__f{}_from_{}{}", fw, u, iw), &mut out);
                        }
                        CastOp::Fptosi | CastOp::Fptoui => {
                            let u = if op == CastOp::Fptoui { "u" } else { "i" };
                            let fw = if f64ty(from) { 64 } else { 32 };
                            let iw = if to == Type::I64 { 64 } else { 32 };
                            call(format!("__f{}_to_{}{}", fw, u, iw), &mut out);
                        }
                        CastOp::Fpromote => call("__f64_from_f32".into(), &mut out),
                        CastOp::Fdemote => call("__f32_from_f64".into(), &mut out),
                        CastOp::Bitcast
                            if from.is_float() || to.is_float() =>
                        {
                            // f64<->i64 / f32<->i32 become identities; a
                            // float<->struct bitcast keeps its (integer)
                            // form after retyping
                            let post = |t: Type| match t {
                                Type::F64 => Type::I64,
                                Type::F32 => Type::I32,
                                t => t,
                            };
                            if post(from) == post(to) {
                                subst.insert(dst, src);
                            } else {
                                out.push(Inst::Cast { op, dst, src });
                            }
                        }
                        _ => out.push(Inst::Cast { op, dst, src }),
                    }
                }
                other => out.push(other),
            }
        }
        func.blocks[b].insts = out;
    }
    if !subst.is_empty() {
        crate::lower::substitute(func, &subst);
    }
    for v in &mut func.values {
        v.ty = match v.ty {
            Type::F64 => Type::I64,
            Type::F32 => Type::I32,
            t => t,
        };
    }
    for r in &mut func.rets {
        *r = match *r {
            Type::F64 => Type::I64,
            Type::F32 => Type::I32,
            t => t,
        };
    }
}

// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// JIT the runtime and hand back callable softfloat ops.
    fn jit() -> crate::emit::jit::JitCode {
        let mut m = ssa::parse(RUNTIME).expect("runtime parses");
        ssa::verify(&m).expect("runtime verifies");
        crate::lower::lower(&mut m);
        crate::opt::optimize(&mut m, crate::opt::MAX_LEVEL);
        ssa::verify(&m).expect("runtime verifies post-lower");
        let enc = crate::emit::Encoder::load("targets/arm64.encodings.json").unwrap();
        let c = crate::emit::compile(&m, &enc).expect("compiles");
        crate::emit::jit::JitCode::new(&c).expect("maps")
    }

    struct Rng(u64);
    impl Rng {
        fn next(&mut self) -> u64 {
            let mut x = self.0;
            x ^= x >> 12;
            x ^= x << 25;
            x ^= x >> 27;
            self.0 = x;
            x.wrapping_mul(0x2545F4914F6CDD1D)
        }
        /// random normal f64 with a mid-range exponent, so results of
        /// +,-,*,/ stay clear of subnormals and infinity
        fn f64_mid(&mut self) -> f64 {
            let sign = self.next() & (1 << 63);
            let exp = (823 + (self.next() % 400)) << 52;
            let man = self.next() & ((1 << 52) - 1);
            f64::from_bits(sign | exp | man)
        }
        fn f32_mid(&mut self) -> f32 {
            let sign = ((self.next() & 1) as u32) << 31;
            let exp = ((77 + (self.next() % 100)) as u32) << 23;
            let man = (self.next() as u32) & ((1 << 23) - 1);
            f32::from_bits(sign | exp | man)
        }
    }

    #[test]
    fn differential_f64_arith_vs_hardware() {
        let j = jit();
        let mut rng = Rng(0x5DEECE66D);
        for i in 0..4000 {
            let (a, b) = (rng.f64_mid(), rng.f64_mid());
            // every few rounds, force near-cancellation to stress normalize
            let b = if i % 5 == 0 {
                f64::from_bits((-a).to_bits() ^ (rng.next() % 4))
            } else {
                b
            };
            let (ab, bb) = (a.to_bits() as i64, b.to_bits() as i64);
            for (name, hw) in [
                ("__f64_add", a + b),
                ("__f64_sub", a - b),
                ("__f64_mul", a * b),
                ("__f64_div", a / b),
            ] {
                let got = j.call(name, &[ab, bb]).unwrap() as u64;
                let want = hw.to_bits();
                // tier-1 scope: hardware subnormal results flush to zero
                let want = if hw != 0.0 && hw.abs() < f64::MIN_POSITIVE {
                    hw.to_bits() & (1 << 63)
                } else {
                    want
                };
                assert_eq!(
                    got, want,
                    "{}({:e}, {:e}): soft {:016x} hw {:016x}",
                    name, a, b, got, want
                );
            }
        }
    }

    #[test]
    fn differential_f64_cmp_and_convert() {
        let j = jit();
        let mut rng = Rng(0xABCDEF012345);
        for _ in 0..4000 {
            let (a, b) = (rng.f64_mid(), rng.f64_mid());
            let (ab, bb) = (a.to_bits() as i64, b.to_bits() as i64);
            assert_eq!(j.call("__f64_eq", &[ab, bb]).unwrap(), (a == b) as i64);
            assert_eq!(j.call("__f64_lt", &[ab, bb]).unwrap(), (a < b) as i64);
            assert_eq!(j.call("__f64_le", &[ab, bb]).unwrap(), (a <= b) as i64);
            assert_eq!(j.call("__f64_lt", &[ab, ab]).unwrap(), 0);
            assert_eq!(j.call("__f64_le", &[ab, ab]).unwrap(), 1);

            let n = rng.next() as i64;
            let got = j.call("__f64_from_i64", &[n]).unwrap() as u64;
            assert_eq!(got, (n as f64).to_bits(), "from_i64({})", n);

            // to_i64 on in-range values (scale into +/-2^50)
            let x = rng.f64_mid();
            let x = if x.abs() >= 2f64.powi(50) {
                x % 2f64.powi(50)
            } else {
                x
            };
            assert_eq!(
                j.call("__f64_to_i64", &[x.to_bits() as i64]).unwrap(),
                x as i64,
                "to_i64({:e})",
                x
            );
        }
        // NaN and infinity behavior
        let nan = f64::NAN.to_bits() as i64;
        let one = 1f64.to_bits() as i64;
        assert_eq!(j.call("__f64_eq", &[nan, nan]).unwrap(), 0);
        assert_eq!(j.call("__f64_ne", &[nan, one]).unwrap(), 1);
        let inf = f64::INFINITY.to_bits() as i64;
        let ninf = f64::NEG_INFINITY.to_bits() as i64;
        let s = j.call("__f64_add", &[inf, ninf]).unwrap() as u64;
        assert!(f64::from_bits(s).is_nan(), "inf + -inf must be NaN");
        assert_eq!(j.call("__f64_add", &[inf, one]).unwrap() as u64, f64::INFINITY.to_bits());
    }

    #[test]
    fn differential_f32_vs_hardware() {
        let j = jit();
        let mut rng = Rng(0x1234_5678_9ABC);
        for i in 0..4000 {
            let (a, b) = (rng.f32_mid(), rng.f32_mid());
            let b = if i % 5 == 0 {
                f32::from_bits((-a).to_bits() ^ ((rng.next() % 4) as u32))
            } else {
                b
            };
            let (ab, bb) = (a.to_bits() as i64, b.to_bits() as i64);
            for (name, hw) in [
                ("__f32_add", a + b),
                ("__f32_sub", a - b),
                ("__f32_mul", a * b),
                ("__f32_div", a / b),
            ] {
                let got = j.call(name, &[ab, bb]).unwrap() as u32;
                let want = if hw != 0.0 && hw.abs() < f32::MIN_POSITIVE {
                    hw.to_bits() & (1 << 31)
                } else {
                    hw.to_bits()
                };
                assert_eq!(
                    got, want,
                    "{}({:e}, {:e}): soft {:08x} hw {:08x}",
                    name, a, b, got, want
                );
            }
            // promote / demote round-trip against hardware casts
            assert_eq!(
                j.call("__f64_from_f32", &[ab]).unwrap() as u64,
                (a as f64).to_bits()
            );
            let d = rng.f64_mid();
            let hw = d as f32;
            // tier-1 scope: an f32-subnormal demotion result flushes to zero
            let want = if hw != 0.0 && hw.abs() < f32::MIN_POSITIVE {
                hw.to_bits() & (1 << 31)
            } else {
                hw.to_bits()
            };
            assert_eq!(
                j.call("__f32_from_f64", &[d.to_bits() as i64]).unwrap() as u32,
                want,
                "demote({:e})",
                d
            );
        }
    }
}
