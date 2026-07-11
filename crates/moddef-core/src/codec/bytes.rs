//! Register/byte assembly honoring byte order within a word and word order
//! across words (spec §9). Allocation-free: integer paths assemble straight
//! into a u64; byte paths iterate.

/// Logical byte index -> (register index, take high byte?) after applying
/// word order across `n` words and byte order within each word.
#[inline]
fn locate(i: usize, n: usize, byte_big: bool, word_big: bool) -> (usize, bool) {
    let w = i / 2;
    let reg = if word_big { w } else { n - 1 - w };
    let hi = if byte_big { i % 2 == 0 } else { i % 2 == 1 };
    (reg, hi)
}

/// Byte at logical big-endian position `i` of the normalized byte stream.
#[inline]
pub fn byte_at(regs: &[u16], i: usize, byte_big: bool, word_big: bool) -> u8 {
    let (reg, hi) = locate(i, regs.len(), byte_big, word_big);
    let w = regs[reg];
    if hi {
        (w >> 8) as u8
    } else {
        (w & 0xff) as u8
    }
}

/// Assemble up to the last 8 normalized bytes into a big-endian u64
/// (equivalent of Go's assemble + decodeUint over the trailing window).
pub fn assemble_u64(regs: &[u16], byte_big: bool, word_big: bool) -> u64 {
    let total = regs.len() * 2;
    let start = total.saturating_sub(8);
    let mut v: u64 = 0;
    for i in start..total {
        v = (v << 8) | byte_at(regs, i, byte_big, word_big) as u64;
    }
    v
}

/// Copy the normalized big-endian byte stream into `out`; returns the byte
/// count written (`regs.len()*2`, or None if `out` is too small).
pub fn copy_bytes(regs: &[u16], byte_big: bool, word_big: bool, out: &mut [u8]) -> Option<usize> {
    let total = regs.len() * 2;
    if out.len() < total {
        return None;
    }
    for (i, b) in out.iter_mut().enumerate().take(total) {
        *b = byte_at(regs, i, byte_big, word_big);
    }
    Some(total)
}

/// Split a raw big-endian integer into `out` registers (inverse of
/// [`assemble_u64`]); `out.len()` defines the width.
pub fn words_from_u64(raw: u64, byte_big: bool, word_big: bool, out: &mut [u16]) {
    let n = out.len();
    for reg in out.iter_mut() {
        *reg = 0;
    }
    let total = n * 2;
    let mut v = raw;
    for i in (0..total).rev() {
        let b = (v & 0xff) as u8;
        v >>= 8;
        let (reg, hi) = locate(i, n, byte_big, word_big);
        if hi {
            out[reg] |= (b as u16) << 8;
        } else {
            out[reg] |= b as u16;
        }
    }
}

/// Write a normalized big-endian byte stream into registers.
pub fn words_from_bytes(bytes: &[u8], byte_big: bool, word_big: bool, out: &mut [u16]) {
    let n = out.len();
    for reg in out.iter_mut() {
        *reg = 0;
    }
    for i in 0..(n * 2) {
        let b = *bytes.get(i).unwrap_or(&0);
        let (reg, hi) = locate(i, n, byte_big, word_big);
        if hi {
            out[reg] |= (b as u16) << 8;
        } else {
            out[reg] |= b as u16;
        }
    }
}

#[inline]
pub fn mask_for(bits: u32) -> u64 {
    if bits >= 64 {
        u64::MAX
    } else {
        (1u64 << bits) - 1
    }
}

#[inline]
pub fn sign_extend(v: u64, bits: u32) -> i64 {
    if bits >= 64 {
        return v as i64;
    }
    let sign = 1u64 << (bits - 1);
    if v & sign != 0 {
        (v | !mask_for(bits)) as i64
    } else {
        v as i64
    }
}
