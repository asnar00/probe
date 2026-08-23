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
type $fp = { frac: u52, exp: u11, sign: u1 }
type $fp32 = { frac: u23, exp: u8, sign: u1 }

fn @__fp_qnan() -> u64 {
    %q: u64 = iconst 0x7ff8000000000000
    ret %q
}

fn @__fp_isnan(%b: u64) -> u1 {
    %p: $fp = bitcast %b
    %e: u11 = extract %p, exp
    %f: u52 = extract %p, frac
    %emax: u11 = iconst 2047
    %isemax: u1 = icmp.eq %e, %emax
    %zf: u52 = iconst 0
    %fnz: u1 = icmp.ne %f, %zf
    %x: u64 = ext %isemax
    %y: u64 = ext %fnz
    %z: u64 = and %x, %y
    %r: u1 = trunc %z
    ret %r
}

fn @__fp_zero(%s: u1) -> u64 {
    %e: u11 = iconst 0
    %f: u52 = iconst 0
    %p: $fp = pack %f, %e, %s
    %r: u64 = bitcast %p
    ret %r
}

fn @__fp_inf(%s: u1) -> u64 {
    %e: u11 = iconst 2047
    %f: u52 = iconst 0
    %p: $fp = pack %f, %e, %s
    %r: u64 = bitcast %p
    ret %r
}

fn @__fp_pack(%s: u1, %e: u64, %m: u64) -> u64 {
    %e11: u11 = trunc %e
    %m52: u52 = trunc %m
    %p: $fp = pack %m52, %e11, %s
    %r: u64 = bitcast %p
    ret %r
}

; m56 holds 1.frac plus guard/round/sticky in [2^55, 2^56); round to
; nearest even, overflowing to infinity. Exponents travel as u64: an
; underflowed exponent shows up as a huge value (top bit set) or zero.
fn @__fp_round(%s: u1, %e: u64, %m56: u64) -> u64 {
    %one: u64 = iconst 1
    %three: u64 = iconst 3
    %zero: u64 = iconst 0
    %m: u64 = shr %m56, %three
    %gsh: u64 = iconst 2
    %g0: u64 = shr %m56, %gsh
    %g: u64 = and %g0, %one
    %rs: u64 = and %m56, %three
    %lsb: u64 = and %m, %one
    %any0: u64 = or %rs, %lsb
    %any: u1 = icmp.ne %any0, %zero
    %anyi: u64 = ext %any
    %up: u64 = and %g, %anyi
    %m2: u64 = iadd %m, %up
    %top: u64 = iconst 0x20000000000000
    %ovf: u1 = icmp.eq %m2, %top
    %ef: u64, %mf: u64 = if %ovf {
        %m3: u64 = shr %m2, %one
        %e2: u64 = iadd %e, %one
        yield %e2, %m3
    } else {
        yield %e, %m2
    }
    %hi: u64 = iconst 0x8000000000000000
    %neg: u1 = icmp.ge %ef, %hi
    if %neg {
        %z0: u64 = call @__fp_zero(%s)
        ret %z0
    }
    %uu: u1 = icmp.eq %ef, %zero
    if %uu {
        %z: u64 = call @__fp_zero(%s)
        ret %z
    }
    %emax: u64 = iconst 2047
    %oo: u1 = icmp.ge %ef, %emax
    if %oo {
        %inf: u64 = call @__fp_inf(%s)
        ret %inf
    }
    %r: u64 = call @__fp_pack(%s, %ef, %mf)
    ret %r
}

fn @__f64_add(%a: u64, %b: u64) -> u64 {
    %an: u1 = call @__fp_isnan(%a)
    if %an {
        %q1: u64 = call @__fp_qnan()
        ret %q1
    }
    %bn: u1 = call @__fp_isnan(%b)
    if %bn {
        %q2: u64 = call @__fp_qnan()
        ret %q2
    }
    %pa: $fp = bitcast %a
    %pb: $fp = bitcast %b
    %sa: u1 = extract %pa, sign
    %sb: u1 = extract %pb, sign
    %ea0: u11 = extract %pa, exp
    %eb0: u11 = extract %pb, exp
    %fa0: u52 = extract %pa, frac
    %fb0: u52 = extract %pb, frac
    %ea: u64 = ext %ea0
    %eb: u64 = ext %eb0
    %fa: u64 = ext %fa0
    %fb: u64 = ext %fb0
    %emax: u64 = iconst 2047
    %zero: u64 = iconst 0
    %one: u64 = iconst 1
    %ainf: u1 = icmp.eq %ea, %emax
    if %ainf {
        %binf: u1 = icmp.eq %eb, %emax
        if %binf {
            %same: u1 = icmp.eq %a, %b
            if %same {
                ret %a
            }
            %q3: u64 = call @__fp_qnan()
            ret %q3
        }
        ret %a
    }
    %binf2: u1 = icmp.eq %eb, %emax
    if %binf2 {
        ret %b
    }
    %az: u1 = icmp.eq %ea, %zero
    %bz: u1 = icmp.eq %eb, %zero
    %azi: u64 = ext %az
    %bzi: u64 = ext %bz
    %bothz0: u64 = and %azi, %bzi
    %bothz: u1 = trunc %bothz0
    if %bothz {
        %sai: u64 = ext %sa
        %sbi: u64 = ext %sb
        %sb0: u64 = and %sai, %sbi
        %sr: u1 = trunc %sb0
        %z1: u64 = call @__fp_zero(%sr)
        ret %z1
    }
    if %az {
        ret %b
    }
    if %bz {
        ret %a
    }
    %hid: u64 = iconst 0x10000000000000
    %Ma: u64 = or %fa, %hid
    %Mb: u64 = or %fb, %hid
    ; order operands so the larger magnitude comes first
    %lte: u1 = icmp.lt %ea, %eb
    %eqe: u1 = icmp.eq %ea, %eb
    %ltm: u1 = icmp.lt %Ma, %Mb
    %eqei: u64 = ext %eqe
    %ltmi: u64 = ext %ltm
    %t0: u64 = and %eqei, %ltmi
    %ltei: u64 = ext %lte
    %sw0: u64 = or %ltei, %t0
    %swap: u1 = trunc %sw0
    %sx: u1, %sy: u1, %e1: u64, %M1: u64, %e2: u64, %M2: u64 = if %swap {
        yield %sb, %sa, %eb, %Mb, %ea, %Ma
    } else {
        yield %sa, %sb, %ea, %Ma, %eb, %Mb
    }
    %d: u64 = isub %e1, %e2
    %three: u64 = iconst 3
    %M13: u64 = shl %M1, %three
    %M23: u64 = shl %M2, %three
    %c60: u64 = iconst 60
    %far: u1 = icmp.gt %d, %c60
    %sh: u64, %st: u64 = if %far {
        yield %zero, %one
    } else {
        %sh0: u64 = shr %M23, %d
        %msk0: u64 = shl %one, %d
        %msk: u64 = isub %msk0, %one
        %lost: u64 = and %M23, %msk
        %lnz: u1 = icmp.ne %lost, %zero
        %sti: u64 = ext %lnz
        yield %sh0, %sti
    }
    %sxi: u64 = ext %sx
    %syi: u64 = ext %sy
    %sdif: u64 = xor %sxi, %syi
    %ssame: u1 = icmp.eq %sdif, %zero
    %m56: u64, %ef: u64 = if %ssame {
        %shj: u64 = or %sh, %st
        %sum: u64 = iadd %M13, %shj
        %lim: u64 = iconst 0x100000000000000
        %c: u1 = icmp.ge %sum, %lim
        %mm: u64, %ee: u64 = if %c {
            %lo: u64 = and %sum, %one
            %s1: u64 = shr %sum, %one
            %s2: u64 = or %s1, %lo
            %ep: u64 = iadd %e1, %one
            yield %s2, %ep
        } else {
            yield %sum, %e1
        }
        yield %mm, %ee
    } else {
        %d0: u64 = isub %M13, %sh
        %d1: u64 = isub %d0, %st
        %dj: u64 = or %d1, %st
        %cz: u1 = icmp.eq %dj, %zero
        if %cz {
            %pos: u1 = iconst 0
            %z2: u64 = call @__fp_zero(%pos)
            ret %z2
        }
        %top55: u64 = iconst 0x80000000000000
        %mn: u64, %en: u64 = loop(%mc: u64 = %dj, %ec: u64 = %e1) {
            %ok: u1 = icmp.ge %mc, %top55
            if %ok {
                break %mc, %ec
            }
            %mc2: u64 = shl %mc, %one
            %ec2: u64 = isub %ec, %one
            continue %mc2, %ec2
        }
        yield %mn, %en
    }
    %r: u64 = call @__fp_round(%sx, %ef, %m56)
    ret %r
}

fn @__f64_sub(%a: u64, %b: u64) -> u64 {
    %sgn: u64 = iconst 0x8000000000000000
    %nb: u64 = xor %b, %sgn
    %r: u64 = call @__f64_add(%a, %nb)
    ret %r
}

fn @__f64_mul(%a: u64, %b: u64) -> u64 {
    %an: u1 = call @__fp_isnan(%a)
    if %an {
        %q1: u64 = call @__fp_qnan()
        ret %q1
    }
    %bn: u1 = call @__fp_isnan(%b)
    if %bn {
        %q2: u64 = call @__fp_qnan()
        ret %q2
    }
    %pa: $fp = bitcast %a
    %pb: $fp = bitcast %b
    %sa: u1 = extract %pa, sign
    %sb: u1 = extract %pb, sign
    %ea0: u11 = extract %pa, exp
    %eb0: u11 = extract %pb, exp
    %fa0: u52 = extract %pa, frac
    %fb0: u52 = extract %pb, frac
    %ea: u64 = ext %ea0
    %eb: u64 = ext %eb0
    %fa: u64 = ext %fa0
    %fb: u64 = ext %fb0
    %sai: u64 = ext %sa
    %sbi: u64 = ext %sb
    %sx0: u64 = xor %sai, %sbi
    %sx: u1 = trunc %sx0
    %emax: u64 = iconst 2047
    %zero: u64 = iconst 0
    %az: u1 = icmp.eq %ea, %zero
    %bz: u1 = icmp.eq %eb, %zero
    %ainf: u1 = icmp.eq %ea, %emax
    if %ainf {
        if %bz {
            %q3: u64 = call @__fp_qnan()
            ret %q3
        }
        %i1_: u64 = call @__fp_inf(%sx)
        ret %i1_
    }
    %binf: u1 = icmp.eq %eb, %emax
    if %binf {
        if %az {
            %q4: u64 = call @__fp_qnan()
            ret %q4
        }
        %i2_: u64 = call @__fp_inf(%sx)
        ret %i2_
    }
    if %az {
        %z1: u64 = call @__fp_zero(%sx)
        ret %z1
    }
    if %bz {
        %z2: u64 = call @__fp_zero(%sx)
        ret %z2
    }
    %hid: u64 = iconst 0x10000000000000
    %Ma: u64 = or %fa, %hid
    %Mb: u64 = or %fb, %hid
    %bias: u64 = iconst 1023
    %es: u64 = iadd %ea, %eb
    %e: u64 = isub %es, %bias
    ; 53x53 -> 106-bit product from 32-bit partials
    %m32: u64 = iconst 0xffffffff
    %c32: u64 = iconst 32
    %a0: u64 = and %Ma, %m32
    %a1: u64 = shr %Ma, %c32
    %b0: u64 = and %Mb, %m32
    %b1: u64 = shr %Mb, %c32
    %p00: u64 = imul %a0, %b0
    %p01: u64 = imul %a0, %b1
    %p10: u64 = imul %a1, %b0
    %p11: u64 = imul %a1, %b1
    %p00h: u64 = shr %p00, %c32
    %p01l: u64 = and %p01, %m32
    %p10l: u64 = and %p10, %m32
    %mid0: u64 = iadd %p00h, %p01l
    %mid: u64 = iadd %mid0, %p10l
    %midl: u64 = and %mid, %m32
    %los: u64 = shl %midl, %c32
    %p00l: u64 = and %p00, %m32
    %lo: u64 = or %los, %p00l
    %p01h: u64 = shr %p01, %c32
    %p10h: u64 = shr %p10, %c32
    %midh: u64 = shr %mid, %c32
    %hi0: u64 = iadd %p11, %p01h
    %hi1: u64 = iadd %hi0, %p10h
    %hi: u64 = iadd %hi1, %midh
    ; take 56 bits from the top of the 106-bit product
    %c49: u64 = iconst 49
    %c15: u64 = iconst 15
    %th: u64 = shl %hi, %c15
    %tl: u64 = shr %lo, %c49
    %t0_: u64 = or %th, %tl
    %lm0: u64 = iconst 0x1ffffffffffff
    %lost: u64 = and %lo, %lm0
    %lnz: u1 = icmp.ne %lost, %zero
    %sti: u64 = ext %lnz
    %t: u64 = or %t0_, %sti
    %lim: u64 = iconst 0x100000000000000
    %c: u1 = icmp.ge %t, %lim
    %one: u64 = iconst 1
    %m56: u64, %ef: u64 = if %c {
        %lo1: u64 = and %t, %one
        %t1: u64 = shr %t, %one
        %t2: u64 = or %t1, %lo1
        %e2: u64 = iadd %e, %one
        yield %t2, %e2
    } else {
        yield %t, %e
    }
    %r: u64 = call @__fp_round(%sx, %ef, %m56)
    ret %r
}

fn @__f64_div(%a: u64, %b: u64) -> u64 {
    %an: u1 = call @__fp_isnan(%a)
    if %an {
        %q1: u64 = call @__fp_qnan()
        ret %q1
    }
    %bn: u1 = call @__fp_isnan(%b)
    if %bn {
        %q2: u64 = call @__fp_qnan()
        ret %q2
    }
    %pa: $fp = bitcast %a
    %pb: $fp = bitcast %b
    %sa: u1 = extract %pa, sign
    %sb: u1 = extract %pb, sign
    %ea0: u11 = extract %pa, exp
    %eb0: u11 = extract %pb, exp
    %fa0: u52 = extract %pa, frac
    %fb0: u52 = extract %pb, frac
    %ea: u64 = ext %ea0
    %eb: u64 = ext %eb0
    %fa: u64 = ext %fa0
    %fb: u64 = ext %fb0
    %sai: u64 = ext %sa
    %sbi: u64 = ext %sb
    %sx0: u64 = xor %sai, %sbi
    %sx: u1 = trunc %sx0
    %emax: u64 = iconst 2047
    %zero: u64 = iconst 0
    %az: u1 = icmp.eq %ea, %zero
    %bz: u1 = icmp.eq %eb, %zero
    %ainf: u1 = icmp.eq %ea, %emax
    %binf: u1 = icmp.eq %eb, %emax
    if %ainf {
        if %binf {
            %q3: u64 = call @__fp_qnan()
            ret %q3
        }
        %i1_: u64 = call @__fp_inf(%sx)
        ret %i1_
    }
    if %binf {
        %z1: u64 = call @__fp_zero(%sx)
        ret %z1
    }
    if %bz {
        if %az {
            %q4: u64 = call @__fp_qnan()
            ret %q4
        }
        %i2_: u64 = call @__fp_inf(%sx)
        ret %i2_
    }
    if %az {
        %z2: u64 = call @__fp_zero(%sx)
        ret %z2
    }
    %hid: u64 = iconst 0x10000000000000
    %Ma: u64 = or %fa, %hid
    %Mb: u64 = or %fb, %hid
    %bias: u64 = iconst 1023
    %ed: u64 = isub %ea, %eb
    %e0: u64 = iadd %ed, %bias
    %one: u64 = iconst 1
    %small: u1 = icmp.lt %Ma, %Mb
    %rem0: u64, %e: u64 = if %small {
        %r2: u64 = shl %Ma, %one
        %e1: u64 = isub %e0, %one
        yield %r2, %e1
    } else {
        yield %Ma, %e0
    }
    ; 55 quotient bits by restoring division
    %c55: u64 = iconst 55
    %q: u64, %rem: u64 = loop(%i: u64 = %zero, %qq: u64 = %zero, %rr: u64 = %rem0) {
        %done: u1 = icmp.ge %i, %c55
        if %done {
            break %qq, %rr
        }
        %q1_: u64 = shl %qq, %one
        %ge: u1 = icmp.ge %rr, %Mb
        %q2_: u64, %r2_: u64 = if %ge {
            %rs: u64 = isub %rr, %Mb
            %qo: u64 = or %q1_, %one
            yield %qo, %rs
        } else {
            yield %q1_, %rr
        }
        %r3_: u64 = shl %r2_, %one
        %i2: u64 = iadd %i, %one
        continue %i2, %q2_, %r3_
    }
    %rnz: u1 = icmp.ne %rem, %zero
    %sti: u64 = ext %rnz
    %m0: u64 = shl %q, %one
    %m56: u64 = or %m0, %sti
    %r: u64 = call @__fp_round(%sx, %e, %m56)
    ret %r
}

; ordered comparisons; magnitudes flush subnormals like the arithmetic
fn @__f64_eq(%a: u64, %b: u64) -> u1 {
    %f: u1 = iconst 0
    %t: u1 = iconst 1
    %an: u1 = call @__fp_isnan(%a)
    if %an {
        ret %f
    }
    %bn: u1 = call @__fp_isnan(%b)
    if %bn {
        ret %f
    }
    %nsgn: u64 = iconst 0x7fffffffffffffff
    %ma: u64 = and %a, %nsgn
    %mb: u64 = and %b, %nsgn
    %zero: u64 = iconst 0
    %az: u1 = icmp.eq %ma, %zero
    %bz: u1 = icmp.eq %mb, %zero
    %azi: u64 = ext %az
    %bzi: u64 = ext %bz
    %both0: u64 = and %azi, %bzi
    %both: u1 = trunc %both0
    if %both {
        ret %t
    }
    %r: u1 = icmp.eq %a, %b
    ret %r
}

fn @__f64_ne(%a: u64, %b: u64) -> u1 {
    %t: u1 = iconst 1
    %an: u1 = call @__fp_isnan(%a)
    if %an {
        ret %t
    }
    %bn: u1 = call @__fp_isnan(%b)
    if %bn {
        ret %t
    }
    %e: u1 = call @__f64_eq(%a, %b)
    %ei: u64 = ext %e
    %one: u64 = iconst 1
    %ni: u64 = xor %ei, %one
    %r: u1 = trunc %ni
    ret %r
}

fn @__f64_lt(%a: u64, %b: u64) -> u1 {
    %f: u1 = iconst 0
    %an: u1 = call @__fp_isnan(%a)
    if %an {
        ret %f
    }
    %bn: u1 = call @__fp_isnan(%b)
    if %bn {
        ret %f
    }
    %nsgn: u64 = iconst 0x7fffffffffffffff
    %ma: u64 = and %a, %nsgn
    %mb: u64 = and %b, %nsgn
    %zero: u64 = iconst 0
    %az: u1 = icmp.eq %ma, %zero
    %bz: u1 = icmp.eq %mb, %zero
    %azi: u64 = ext %az
    %bzi: u64 = ext %bz
    %both0: u64 = and %azi, %bzi
    %both: u1 = trunc %both0
    if %both {
        ret %f
    }
    %c63: u64 = iconst 63
    %sa: u64 = shr %a, %c63
    %sb: u64 = shr %b, %c63
    %sdiff: u1 = icmp.ne %sa, %sb
    if %sdiff {
        %r1: u1 = trunc %sa
        ret %r1
    }
    %sneg: u1 = trunc %sa
    if %sneg {
        %r2: u1 = icmp.gt %ma, %mb
        ret %r2
    }
    %r3: u1 = icmp.lt %ma, %mb
    ret %r3
}

fn @__f64_le(%a: u64, %b: u64) -> u1 {
    %f: u1 = iconst 0
    %an: u1 = call @__fp_isnan(%a)
    if %an {
        ret %f
    }
    %bn: u1 = call @__fp_isnan(%b)
    if %bn {
        ret %f
    }
    %lt: u1 = call @__f64_lt(%b, %a)
    %li: u64 = ext %lt
    %one: u64 = iconst 1
    %ni: u64 = xor %li, %one
    %r: u1 = trunc %ni
    ret %r
}

fn @__f64_from_i64(%n: i64) -> u64 {
    %zeroi: i64 = iconst 0
    %zero: u64 = iconst 0
    %isz: u1 = icmp.eq %n, %zeroi
    if %isz {
        ret %zero
    }
    %neg: u1 = icmp.lt %n, %zeroi
    %magi: i64 = if %neg {
        %m0: i64 = isub %zeroi, %n
        yield %m0
    } else {
        yield %n
    }
    %mag: u64 = bitcast %magi
    ; find the highest set bit
    %c63: u64 = iconst 63
    %one: u64 = iconst 1
    %hb: u64 = loop(%i: u64 = %c63) {
        %sh: u64 = shr %mag, %i
        %bit: u64 = and %sh, %one
        %set: u1 = icmp.ne %bit, %zero
        if %set {
            break %i
        }
        %i2: u64 = isub %i, %one
        continue %i2
    }
    %bias: u64 = iconst 1023
    %e: u64 = iadd %bias, %hb
    %c52: u64 = iconst 52
    %fits: u1 = icmp.le %hb, %c52
    if %fits {
        %shl_: u64 = isub %c52, %hb
        %m: u64 = shl %mag, %shl_
        %r1: u64 = call @__fp_pack(%neg, %e, %m)
        ret %r1
    }
    %sh2: u64 = isub %hb, %c52
    %keep: u64 = shr %mag, %sh2
    %gsh: u64 = isub %sh2, %one
    %g0: u64 = shr %mag, %gsh
    %g: u64 = and %g0, %one
    %rm0: u64 = shl %one, %gsh
    %rm: u64 = isub %rm0, %one
    %rest: u64 = and %mag, %rm
    %rnz: u1 = icmp.ne %rest, %zero
    %sti: u64 = ext %rnz
    %lsb: u64 = and %keep, %one
    %any0: u64 = or %sti, %lsb
    %anynz: u1 = icmp.ne %any0, %zero
    %anyi: u64 = ext %anynz
    %up: u64 = and %g, %anyi
    %m2: u64 = iadd %keep, %up
    %top: u64 = iconst 0x20000000000000
    %ovf: u1 = icmp.eq %m2, %top
    %ef: u64, %mf: u64 = if %ovf {
        %m3: u64 = shr %m2, %one
        %e2: u64 = iadd %e, %one
        yield %e2, %m3
    } else {
        yield %e, %m2
    }
    %r2: u64 = call @__fp_pack(%neg, %ef, %mf)
    ret %r2
}

fn @__f64_from_u64(%n: u64) -> u64 {
    %top: u64 = iconst 0x8000000000000000
    %big: u1 = icmp.ge %n, %top
    if %big {
        ; halve with the lost bit jammed, convert, double back — exact
        %one: u64 = iconst 1
        %h0: u64 = shr %n, %one
        %lo: u64 = and %n, %one
        %h: u64 = or %h0, %lo
        %hi_: i64 = bitcast %h
        %d: u64 = call @__f64_from_i64(%hi_)
        %two: u64 = iconst 0x4000000000000000
        %r0: u64 = call @__f64_mul(%d, %two)
        ret %r0
    }
    %ni: i64 = bitcast %n
    %r: u64 = call @__f64_from_i64(%ni)
    ret %r
}

fn @__f64_to_i64(%a: u64) -> i64 {
    %zeroi: i64 = iconst 0
    %an: u1 = call @__fp_isnan(%a)
    if %an {
        ret %zeroi
    }
    %p: $fp = bitcast %a
    %s: u1 = extract %p, sign
    %e0: u11 = extract %p, exp
    %f0: u52 = extract %p, frac
    %e: u64 = ext %e0
    %f: u64 = ext %f0
    %bias: u64 = iconst 1023
    %lt1: u1 = icmp.lt %e, %bias
    if %lt1 {
        ret %zeroi
    }
    %sh: u64 = isub %e, %bias
    %c62: u64 = iconst 62
    %huge: u1 = icmp.gt %sh, %c62
    if %huge {
        if %s {
            %min: i64 = iconst 0x8000000000000000
            ret %min
        }
        %max: i64 = iconst 0x7fffffffffffffff
        ret %max
    }
    %hid: u64 = iconst 0x10000000000000
    %M: u64 = or %f, %hid
    %c52: u64 = iconst 52
    %left: u1 = icmp.gt %sh, %c52
    %r0: u64 = if %left {
        %k1: u64 = isub %sh, %c52
        %v1: u64 = shl %M, %k1
        yield %v1
    } else {
        %k2: u64 = isub %c52, %sh
        %v2: u64 = shr %M, %k2
        yield %v2
    }
    %ri: i64 = bitcast %r0
    if %s {
        %nn: i64 = isub %zeroi, %ri
        ret %nn
    }
    ret %ri
}

fn @__f64_to_u64(%a: u64) -> u64 {
    %zero: u64 = iconst 0
    %c63: u64 = iconst 63
    %sa: u64 = shr %a, %c63
    %neg: u1 = trunc %sa
    if %neg {
        ret %zero
    }
    %ri: i64 = call @__f64_to_i64(%a)
    %r: u64 = bitcast %ri
    ret %r
}

fn @__f64_from_i32(%n: i32) -> u64 {
    %w: i64 = ext %n
    %r: u64 = call @__f64_from_i64(%w)
    ret %r
}

fn @__f64_from_u32(%n: u32) -> u64 {
    %w0: u64 = ext %n
    %w: i64 = bitcast %w0
    %r: u64 = call @__f64_from_i64(%w)
    ret %r
}

fn @__f64_to_i32(%a: u64) -> i32 {
    %w: i64 = call @__f64_to_i64(%a)
    %r: i32 = trunc %w
    ret %r
}

fn @__f64_to_u32(%a: u64) -> u32 {
    %w: u64 = call @__f64_to_u64(%a)
    %r: u32 = trunc %w
    ret %r
}

fn @__f64_from_f32(%b: u32) -> u64 {
    %p: $fp32 = bitcast %b
    %s: u1 = extract %p, sign
    %e0: u8 = extract %p, exp
    %f0: u23 = extract %p, frac
    %e8: u64 = ext %e0
    %f: u64 = ext %f0
    %c255: u64 = iconst 255
    %zero: u64 = iconst 0
    %isinfnan: u1 = icmp.eq %e8, %c255
    if %isinfnan {
        %fz: u1 = icmp.eq %f, %zero
        if %fz {
            %inf: u64 = call @__fp_inf(%s)
            ret %inf
        }
        %q: u64 = call @__fp_qnan()
        ret %q
    }
    %isz: u1 = icmp.eq %e8, %zero
    if %isz {
        %z: u64 = call @__fp_zero(%s)
        ret %z
    }
    %c896: u64 = iconst 896
    %e: u64 = iadd %e8, %c896
    %c29: u64 = iconst 29
    %m: u64 = shl %f, %c29
    %r: u64 = call @__fp_pack(%s, %e, %m)
    ret %r
}

fn @__f32_from_f64(%a: u64) -> u32 {
    %an: u1 = call @__fp_isnan(%a)
    if %an {
        %q: u32 = iconst 0x7fc00000
        ret %q
    }
    %p: $fp = bitcast %a
    %s: u1 = extract %p, sign
    %e0: u11 = extract %p, exp
    %f0: u52 = extract %p, frac
    %e: u64 = ext %e0
    %f: u64 = ext %f0
    %si: u64 = ext %s
    %c31: u64 = iconst 31
    %stop: u64 = shl %si, %c31
    %sbits: u32 = trunc %stop
    %emax: u64 = iconst 2047
    %zero: u64 = iconst 0
    %isinf: u1 = icmp.eq %e, %emax
    if %isinf {
        %infc: u32 = iconst 0x7f800000
        %r0: u32 = or %sbits, %infc
        ret %r0
    }
    %isz: u1 = icmp.eq %e, %zero
    if %isz {
        ret %sbits
    }
    %c896: u64 = iconst 896
    %e32: u64 = isub %e, %c896
    %hibit: u64 = iconst 0x8000000000000000
    %tooSmall0: u1 = icmp.ge %e32, %hibit
    if %tooSmall0 {
        ret %sbits
    }
    %tooSmall1: u1 = icmp.eq %e32, %zero
    if %tooSmall1 {
        ret %sbits
    }
    %c255: u64 = iconst 255
    %tooBig: u1 = icmp.ge %e32, %c255
    if %tooBig {
        %infc2: u32 = iconst 0x7f800000
        %r1: u32 = or %sbits, %infc2
        ret %r1
    }
    %c29: u64 = iconst 29
    %keep: u64 = shr %f, %c29
    %c28: u64 = iconst 28
    %g0: u64 = shr %f, %c28
    %one: u64 = iconst 1
    %g: u64 = and %g0, %one
    %rm: u64 = iconst 0xfffffff
    %rest: u64 = and %f, %rm
    %rnz: u1 = icmp.ne %rest, %zero
    %sti: u64 = ext %rnz
    %lsb: u64 = and %keep, %one
    %any0: u64 = or %sti, %lsb
    %anynz: u1 = icmp.ne %any0, %zero
    %anyi: u64 = ext %anynz
    %up: u64 = and %g, %anyi
    %m2: u64 = iadd %keep, %up
    %m23top: u64 = iconst 0x800000
    %ovf: u1 = icmp.eq %m2, %m23top
    %ef: u64, %mf: u64 = if %ovf {
        %e2: u64 = iadd %e32, %one
        yield %e2, %zero
    } else {
        yield %e32, %m2
    }
    %again: u1 = icmp.ge %ef, %c255
    if %again {
        %infc3: u32 = iconst 0x7f800000
        %r2: u32 = or %sbits, %infc3
        ret %r2
    }
    %e8: u8 = trunc %ef
    %m23: u23 = trunc %mf
    %sp: $fp32 = pack %m23, %e8, %s
    %r: u32 = bitcast %sp
    ret %r
}

; f32 arithmetic: promote, operate in f64, demote — correctly rounded
; because 53 >= 2*24 + 2 (innocuous double rounding)
fn @__f32_add(%a: u32, %b: u32) -> u32 {
    %wa: u64 = call @__f64_from_f32(%a)
    %wb: u64 = call @__f64_from_f32(%b)
    %w: u64 = call @__f64_add(%wa, %wb)
    %r: u32 = call @__f32_from_f64(%w)
    ret %r
}

fn @__f32_sub(%a: u32, %b: u32) -> u32 {
    %wa: u64 = call @__f64_from_f32(%a)
    %wb: u64 = call @__f64_from_f32(%b)
    %w: u64 = call @__f64_sub(%wa, %wb)
    %r: u32 = call @__f32_from_f64(%w)
    ret %r
}

fn @__f32_mul(%a: u32, %b: u32) -> u32 {
    %wa: u64 = call @__f64_from_f32(%a)
    %wb: u64 = call @__f64_from_f32(%b)
    %w: u64 = call @__f64_mul(%wa, %wb)
    %r: u32 = call @__f32_from_f64(%w)
    ret %r
}

fn @__f32_div(%a: u32, %b: u32) -> u32 {
    %wa: u64 = call @__f64_from_f32(%a)
    %wb: u64 = call @__f64_from_f32(%b)
    %w: u64 = call @__f64_div(%wa, %wb)
    %r: u32 = call @__f32_from_f64(%w)
    ret %r
}

fn @__f32_eq(%a: u32, %b: u32) -> u1 {
    %wa: u64 = call @__f64_from_f32(%a)
    %wb: u64 = call @__f64_from_f32(%b)
    %r: u1 = call @__f64_eq(%wa, %wb)
    ret %r
}

fn @__f32_ne(%a: u32, %b: u32) -> u1 {
    %wa: u64 = call @__f64_from_f32(%a)
    %wb: u64 = call @__f64_from_f32(%b)
    %r: u1 = call @__f64_ne(%wa, %wb)
    ret %r
}

fn @__f32_lt(%a: u32, %b: u32) -> u1 {
    %wa: u64 = call @__f64_from_f32(%a)
    %wb: u64 = call @__f64_from_f32(%b)
    %r: u1 = call @__f64_lt(%wa, %wb)
    ret %r
}

fn @__f32_le(%a: u32, %b: u32) -> u1 {
    %wa: u64 = call @__f64_from_f32(%a)
    %wb: u64 = call @__f64_from_f32(%b)
    %r: u1 = call @__f64_le(%wa, %wb)
    ret %r
}

fn @__f32_from_i64(%n: i64) -> u32 {
    %w: u64 = call @__f64_from_i64(%n)
    %r: u32 = call @__f32_from_f64(%w)
    ret %r
}

fn @__f32_from_u64(%n: u64) -> u32 {
    %w: u64 = call @__f64_from_u64(%n)
    %r: u32 = call @__f32_from_f64(%w)
    ret %r
}

fn @__f32_from_i32(%n: i32) -> u32 {
    %w: u64 = call @__f64_from_i32(%n)
    %r: u32 = call @__f32_from_f64(%w)
    ret %r
}

fn @__f32_from_u32(%n: u32) -> u32 {
    %w: u64 = call @__f64_from_u32(%n)
    %r: u32 = call @__f32_from_f64(%w)
    ret %r
}

fn @__f32_to_i64(%a: u32) -> i64 {
    %w: u64 = call @__f64_from_f32(%a)
    %r: i64 = call @__f64_to_i64(%w)
    ret %r
}

fn @__f32_to_u64(%a: u32) -> u64 {
    %w: u64 = call @__f64_from_f32(%a)
    %r: u64 = call @__f64_to_u64(%w)
    ret %r
}

fn @__f32_to_i32(%a: u32) -> i32 {
    %w: i64 = call @__f32_to_i64(%a)
    %r: i32 = trunc %w
    ret %r
}

fn @__f32_to_u32(%a: u32) -> u32 {
    %w: u64 = call @__f32_to_u64(%a)
    %r: u32 = trunc %w
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
    // scalarize vectors first so float lanes arrive as scalar float ops
    crate::lower::lower_vectors(module);
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
                        CastOp::Itof => {
                            let u = if from.is_signed() { "i" } else { "u" };
                            let fw = if f64ty(to) { 64 } else { 32 };
                            let iw = if from.width() == Some(64) { 64 } else { 32 };
                            call(format!("__f{}_from_{}{}", fw, u, iw), &mut out);
                        }
                        CastOp::Ftoi => {
                            let u = if to.is_signed() { "i" } else { "u" };
                            let fw = if f64ty(from) { 64 } else { 32 };
                            let iw = if to.width() == Some(64) { 64 } else { 32 };
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
                                Type::F64 => Type::U(64),
                                Type::F32 => Type::U(32),
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
    // softened float values are raw bit patterns: unsigned
    for v in &mut func.values {
        v.ty = match v.ty {
            Type::F64 => Type::U(64),
            Type::F32 => Type::U(32),
            t => t,
        };
    }
    for r in &mut func.rets {
        *r = match *r {
            Type::F64 => Type::U(64),
            Type::F32 => Type::U(32),
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
