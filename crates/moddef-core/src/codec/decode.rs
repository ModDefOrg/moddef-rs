// SPDX-License-Identifier: Apache-2.0

//! Point decoder (spec §8–§15), allocation-free. Port of go/codec/decode.go
//! and moddef-ts decode.ts (incl. §10.5 selector cases with transform
//! fallback), operating on [`PointDesc`].

use crate::codec::bytes::{assemble_u64, byte_at, copy_bytes, mask_for, sign_extend};
use crate::codec::rat::Rat;
use crate::desc::{
    DateTimeEncoding, PointDesc, ScaleMode, StorageType, StringPadding, StringTermination,
    ValueKind,
};
use crate::error::DecodeError;
use crate::value::Value;

/// Cross-point context: integer values of scale_ref / selector_ref targets
/// (spec §10.4/§10.5). A slice keeps it heap-free; lookups are O(n) over a
/// handful of refs.
#[derive(Clone, Copy, Debug, Default)]
pub struct Ctx<'a> {
    pub refs: &'a [(&'a str, i64)],
}

impl<'a> Ctx<'a> {
    pub const EMPTY: Ctx<'static> = Ctx { refs: &[] };

    pub fn get(&self, id: &str) -> Option<i64> {
        self.refs.iter().find(|(k, _)| *k == id).map(|(_, v)| *v)
    }
}

/// Decode a point's registers into a heap-free [`Value`].
///
/// Strings/bytes are not handled here — use [`decode_str`] / [`decode_bytes`]
/// with a caller buffer (no_std) or the `alloc` facade.
pub fn decode(p: &PointDesc<'_>, regs: &[u16], ctx: &Ctx<'_>) -> Result<Value, DecodeError> {
    if regs.len() < p.words() {
        return Err(DecodeError::ShortRead);
    }
    let regs = &regs[..p.words()];

    // §14 composed mantissa/exponent over the window.
    if let ValueKind::Composed {
        base,
        mantissa_offset,
        mantissa_words,
        exponent_offset,
        exponent_words,
    } = p.value
    {
        if base == 0 {
            return Err(DecodeError::ComposedBaseZero);
        }
        let mant = sub_int(
            regs,
            mantissa_offset,
            mantissa_words,
            p.byte_big,
            p.word_big,
        );
        let exp = sub_int(
            regs,
            exponent_offset,
            exponent_words,
            p.byte_big,
            p.word_big,
        );
        let mut r = Rat::int(mant);
        let b = Rat::int(base);
        if exp >= 0 {
            for _ in 0..exp.min(64) {
                r = r.mul(b);
            }
        } else {
            for _ in 0..(-exp).min(64) {
                r = r.div(b);
            }
        }
        return Ok(Value::F64(r.to_f64()));
    }

    // IEEE754 floats decode straight from the normalized byte stream.
    if p.storage == StorageType::F32 {
        let raw = assemble_u64(regs, p.byte_big, p.word_big) as u32;
        return Ok(Value::F64(apply_float_scale(f32::from_bits(raw) as f64, p)));
    }
    if p.storage == StorageType::F64 {
        let raw = assemble_u64(regs, p.byte_big, p.word_big);
        return Ok(Value::F64(apply_float_scale(f64::from_bits(raw), p)));
    }

    // Integer-backed value.
    let bits = p.storage.bits(regs.len());
    let raw = assemble_u64(regs, p.byte_big, p.word_big) & mask_for(bits);

    // §8.4 sentinel check on the masked raw integer.
    for na in p.na {
        if (na.raw as u64) & mask_for(bits) == raw {
            return Ok(Value::Unavailable);
        }
    }

    // §13.2 flags / §13 fields surface the raw window.
    match p.value {
        ValueKind::Flags(_) => return Ok(Value::Flags(raw)),
        ValueKind::Fields(_) => return Ok(Value::Fields(raw)),
        _ => {}
    }

    if p.storage == StorageType::Bcd {
        return Ok(Value::I64(bcd_to_int(regs, p.byte_big, p.word_big)));
    }

    let signed = p.storage.signed();
    let raw_int: i64 = if signed {
        sign_extend(raw, bits)
    } else {
        raw as i64
    };

    match p.value {
        ValueKind::Bool => Ok(Value::Bool(raw != 0)),
        ValueKind::DateTime(enc) => Ok(Value::DateTime(match enc {
            DateTimeEncoding::EpochSeconds | DateTimeEncoding::EpochMillis => raw as i64,
        })),
        ValueKind::Decimal => Ok(Value::F64(apply_scale(raw_int, signed, raw, p, ctx)?)),
        ValueKind::Uint => Ok(Value::U64(raw)),
        ValueKind::Int => Ok(Value::I64(raw_int)),
        // Enum-backed and anything else integer-shaped: raw integer, signed
        // if the storage is (parity with Go/TS default branch).
        _ => Ok(if signed {
            Value::I64(raw_int)
        } else {
            Value::U64(raw)
        }),
    }
}

/// Pre-scale integer view (exactness escape hatch, parity with TS
/// `decodePointRaw`).
pub fn decode_raw(p: &PointDesc<'_>, regs: &[u16]) -> Result<(u64, u32), DecodeError> {
    if regs.len() < p.words() {
        return Err(DecodeError::ShortRead);
    }
    let regs = &regs[..p.words()];
    let bits = p.storage.bits(regs.len());
    Ok((
        assemble_u64(regs, p.byte_big, p.word_big) & mask_for(bits),
        bits,
    ))
}

/// Decode a string point into a caller-provided buffer (§15).
pub fn decode_str<'b>(
    p: &PointDesc<'_>,
    regs: &[u16],
    out: &'b mut [u8],
) -> Result<&'b str, DecodeError> {
    if regs.len() < p.words() {
        return Err(DecodeError::ShortRead);
    }
    let regs = &regs[..p.words()];
    let n = copy_bytes(regs, p.byte_big, p.word_big, out).ok_or(DecodeError::BufferTooSmall)?;
    let (padding, termination) = match p.value {
        ValueKind::Str {
            padding,
            termination,
        } => (padding, termination),
        _ => (StringPadding::None, StringTermination::FixedLength),
    };
    let mut end = n;
    if termination == StringTermination::NullTerminated {
        if let Some(i) = out[..n].iter().position(|&b| b == 0) {
            end = i;
        }
    }
    let pad = match padding {
        StringPadding::Null => Some(0u8),
        StringPadding::Space => Some(b' '),
        StringPadding::None => None,
    };
    if let Some(c) = pad {
        while end > 0 && out[end - 1] == c {
            end -= 1;
        }
    }
    core::str::from_utf8(&out[..end]).map_err(|_| DecodeError::InvalidUtf8)
}

/// Decode a BYTES_RAW point into a caller-provided buffer.
pub fn decode_bytes<'b>(
    p: &PointDesc<'_>,
    regs: &[u16],
    out: &'b mut [u8],
) -> Result<&'b [u8], DecodeError> {
    if regs.len() < p.words() {
        return Err(DecodeError::ShortRead);
    }
    let regs = &regs[..p.words()];
    let n = copy_bytes(regs, p.byte_big, p.word_big, out).ok_or(DecodeError::BufferTooSmall)?;
    Ok(&out[..n])
}

/// §10 transform pipeline (static rational, scale_ref, selector cases).
fn apply_scale(
    raw_int: i64,
    signed: bool,
    raw_u: u64,
    p: &PointDesc<'_>,
    ctx: &Ctx<'_>,
) -> Result<f64, DecodeError> {
    let mut r = if signed {
        Rat::int(raw_int)
    } else {
        Rat::from_u64(raw_u)
    };

    // §10.5: a matching selector case replaces the point's own transform;
    // unresolved selector or unmatched case falls through (Go/TS parity).
    if let Some(sel) = &p.selector {
        if let Some(key) = ctx.get(sel.point_id) {
            if let Some(c) = sel.cases.iter().find(|c| c.key == key) {
                if let Some(s) = c.scale {
                    if s.den != 0 {
                        r = r.mul(Rat::new(s.num as i128, s.den as i128));
                    }
                }
                if let Some(o) = c.offset {
                    if o.den != 0 {
                        r = r.add(Rat::new(o.num as i128, o.den as i128));
                    }
                }
                return Ok(r.to_f64());
            }
        }
    }

    if let Some(sr) = &p.scale_ref {
        let sf = ctx.get(sr.point_id).ok_or(DecodeError::UnresolvedRef)?;
        match sr.mode {
            ScaleMode::Pow10 => r = r.mul(Rat::pow10(sf)),
            ScaleMode::Multiply => {
                let den = if sr.denominator == 0 {
                    1
                } else {
                    sr.denominator
                };
                r = r.mul(Rat::new(sf as i128, den as i128));
            }
        }
    } else if let Some(s) = p.scale {
        if s.den == 0 {
            return Err(DecodeError::ZeroScaleDenominator);
        }
        r = r.mul(Rat::new(s.num as i128, s.den as i128));
    }

    if let Some(o) = p.offset_add {
        if o.den != 0 {
            r = r.add(Rat::new(o.num as i128, o.den as i128));
        }
    }
    Ok(r.to_f64())
}

fn apply_float_scale(mut f: f64, p: &PointDesc<'_>) -> f64 {
    if let Some(s) = p.scale {
        if s.den != 0 {
            f = f * s.num as f64 / s.den as f64;
        }
    }
    if let Some(o) = p.offset_add {
        if o.den != 0 {
            f += o.num as f64 / o.den as f64;
        }
    }
    f
}

/// Signed integer from a sub-window (composed mantissa/exponent, §14).
fn sub_int(regs: &[u16], offset: u16, words: u8, byte_big: bool, word_big: bool) -> i64 {
    let idx = offset as usize;
    let n = if words == 0 { 1 } else { words as usize };
    if idx + n > regs.len() {
        return 0;
    }
    let slice = &regs[idx..idx + n];
    let raw = assemble_u64(slice, byte_big, word_big);
    sign_extend(raw & mask_for((n * 16) as u32), (n * 16) as u32)
}

fn bcd_to_int(regs: &[u16], byte_big: bool, word_big: bool) -> i64 {
    let mut v: i64 = 0;
    for i in 0..regs.len() * 2 {
        let b = byte_at(regs, i, byte_big, word_big);
        v = v * 100 + ((b >> 4) as i64) * 10 + (b & 0x0f) as i64;
    }
    v
}
