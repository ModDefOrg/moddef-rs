//! Point encoder (spec §10 inverse, §11.4/§11.5), allocation-free. Port of
//! go/codec/encode.go / moddef-ts encode.ts: composed values and packed
//! field windows stay read-oriented.

use crate::codec::bytes::{mask_for, words_from_bytes, words_from_u64};
use crate::codec::decode::Ctx;
use crate::codec::rat::Rat;
use crate::desc::{PointDesc, ScaleMode, StorageType, ValueKind};
use crate::error::{ConstraintKind, EncodeError};
use crate::value::Value;

/// Encode a value into `out` registers (must be exactly `p.words()` long).
pub fn encode(
    p: &PointDesc<'_>,
    v: &Value,
    ctx: &Ctx<'_>,
    out: &mut [u16],
) -> Result<(), EncodeError> {
    if out.len() < p.words() {
        return Err(EncodeError::BufferTooSmall);
    }
    let out = &mut out[..p.words()];

    match p.storage {
        StorageType::Composed => return Err(EncodeError::Unsupported),
        StorageType::F32 => {
            let f = v.as_f64().ok_or(EncodeError::WrongValueType)? as f32;
            words_from_u64(f.to_bits() as u64, p.byte_big, p.word_big, out);
            return Ok(());
        }
        StorageType::F64 => {
            let f = v.as_f64().ok_or(EncodeError::WrongValueType)?;
            words_from_u64(f.to_bits(), p.byte_big, p.word_big, out);
            return Ok(());
        }
        StorageType::StringAscii | StorageType::StringUtf8 | StorageType::BytesRaw => {
            return Err(EncodeError::WrongValueType); // use encode_str / encode_bytes
        }
        _ => {}
    }

    let bits = p.storage.bits(out.len());
    let raw: u64 = match (&p.value, v) {
        (ValueKind::Flags(_), Value::Flags(mask)) => *mask & mask_for(bits),
        (ValueKind::Fields(_), _) => return Err(EncodeError::Unsupported),
        (ValueKind::Bool, v) => (v.as_i64().ok_or(EncodeError::WrongValueType)? != 0) as u64,
        (ValueKind::DateTime(_), Value::DateTime(t)) => *t as u64 & mask_for(bits),
        (ValueKind::Decimal, v) => {
            let val = match v {
                Value::F64(f) => Rat::from_f64(*f),
                Value::I64(i) => Rat::int(*i),
                Value::U64(u) => Rat::from_u64(*u),
                _ => return Err(EncodeError::WrongValueType),
            };
            (encode_scaled(val, p, ctx)? as u64) & mask_for(bits)
        }
        (_, v) => {
            let i = v.as_i64().ok_or(EncodeError::WrongValueType)?;
            (i as u64) & mask_for(bits)
        }
    };

    let raw = if p.storage == StorageType::Bcd {
        int_to_bcd(raw as i64)
    } else {
        raw
    };
    words_from_u64(raw, p.byte_big, p.word_big, out);
    Ok(())
}

/// Encode a string point (fixed window, §15); pads with zero bytes.
pub fn encode_str(p: &PointDesc<'_>, s: &str, out: &mut [u16]) -> Result<(), EncodeError> {
    if out.len() < p.words() {
        return Err(EncodeError::BufferTooSmall);
    }
    words_from_bytes(s.as_bytes(), p.byte_big, p.word_big, &mut out[..p.words()]);
    Ok(())
}

/// Encode raw bytes (BYTES_RAW).
pub fn encode_bytes(p: &PointDesc<'_>, b: &[u8], out: &mut [u16]) -> Result<(), EncodeError> {
    if out.len() < p.words() {
        return Err(EncodeError::BufferTooSmall);
    }
    words_from_bytes(b, p.byte_big, p.word_big, &mut out[..p.words()]);
    Ok(())
}

/// Inverse transform pipeline: raw = (value - offset) / scale.
fn encode_scaled(mut val: Rat, p: &PointDesc<'_>, ctx: &Ctx<'_>) -> Result<i64, EncodeError> {
    if let Some(o) = p.offset_add {
        if o.den != 0 {
            val = val.sub(Rat::new(o.num as i128, o.den as i128));
        }
    }
    if let Some(sr) = &p.scale_ref {
        let sf = ctx.get(sr.point_id).ok_or(EncodeError::UnresolvedRef)?;
        match sr.mode {
            ScaleMode::Pow10 => val = val.div(Rat::pow10(sf)),
            ScaleMode::Multiply => {
                let den = if sr.denominator == 0 {
                    1
                } else {
                    sr.denominator
                };
                val = val.div(Rat::new(sf as i128, den as i128));
            }
        }
    } else if let Some(s) = p.scale {
        if s.den != 0 {
            val = val.div(Rat::new(s.num as i128, s.den as i128));
        }
    }
    Ok(val.round())
}

/// §11.4 write constraint validation in engineering units.
pub fn validate_write(p: &PointDesc<'_>, v: &Value) -> Result<(), ConstraintKind> {
    let Some(w) = &p.write else { return Ok(()) };
    let num = v.as_f64();

    if !w.allowed.is_empty() {
        let iv = num.map(|f| libm_round(f) as i64);
        match iv {
            Some(iv) if w.allowed.contains(&iv) => return Ok(()),
            _ => return Err(ConstraintKind::AllowedValues),
        }
    }
    let Some(num) = num else { return Ok(()) };
    if let Some(min) = w.min {
        if min.den != 0 && num < min.num as f64 / min.den as f64 {
            return Err(ConstraintKind::Min);
        }
    }
    if let Some(max) = w.max {
        if max.den != 0 && num > max.num as f64 / max.den as f64 {
            return Err(ConstraintKind::Max);
        }
    }
    if let Some(step) = w.step {
        if step.den != 0 && step.num != 0 {
            let s = step.num as f64 / step.den as f64;
            let base = w
                .min
                .filter(|m| m.den != 0)
                .map(|m| m.num as f64 / m.den as f64)
                .unwrap_or(0.0);
            let k = (num - base) / s;
            if (k - libm_round(k)).abs() > 1e-9 {
                return Err(ConstraintKind::Step);
            }
        }
    }
    Ok(())
}

fn int_to_bcd(mut v: i64) -> u64 {
    let mut raw: u64 = 0;
    let mut shift = 0;
    while v > 0 {
        raw |= ((v % 10) as u64) << shift;
        shift += 4;
        v /= 10;
    }
    raw
}

/// f64::round is not available in core on all channels historically; a tiny
/// half-away-from-zero rounding that works in no_std.
#[inline]
fn libm_round(f: f64) -> f64 {
    if f >= 0.0 {
        (f + 0.5) as i64 as f64
    } else {
        (f - 0.5) as i64 as f64
    }
}
