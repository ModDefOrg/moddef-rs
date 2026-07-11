//! Runtime support for generated clients (spec §31). `no_std`, allocation
//! free: generated code (moddef-codegen) drives the codec core over its
//! `static POINTS` table and calls into these helpers so the emitted method
//! bodies stay small. Not a stable public API surface — pinned by the
//! generator, may change with it.

use crate::codec::{decode_raw, mask_for};
use crate::desc::{AddressSpace, PointDesc};
use crate::error::Error;
use crate::transport::Transport;
use crate::value::Value;

/// Read a point window into a fixed-size register array. Coil/discrete
/// spaces read a single bit into word 0 (parity with the dynamic facade).
pub async fn read_regs<T: Transport, const N: usize>(
    t: &mut T,
    space: AddressSpace,
    off: u16,
) -> Result<[u16; N], Error<T::Error>> {
    let mut regs = [0u16; N];
    match space {
        AddressSpace::HoldingRegister => t
            .read_holding(off, &mut regs)
            .await
            .map_err(Error::Transport)?,
        AddressSpace::InputRegister => t
            .read_input(off, &mut regs)
            .await
            .map_err(Error::Transport)?,
        AddressSpace::Coil => {
            let mut bits = [false];
            t.read_coils(off, &mut bits)
                .await
                .map_err(Error::Transport)?;
            regs[0] = bits[0] as u16;
        }
        AddressSpace::DiscreteInput => {
            let mut bits = [false];
            t.read_discrete(off, &mut bits)
                .await
                .map_err(Error::Transport)?;
            regs[0] = bits[0] as u16;
        }
    }
    Ok(regs)
}

/// Write encoded registers (holding) or a single coil.
pub async fn write_regs<T: Transport>(
    t: &mut T,
    space: AddressSpace,
    off: u16,
    regs: &[u16],
) -> Result<(), Error<T::Error>> {
    match space {
        AddressSpace::HoldingRegister => t.write_holding(off, regs).await.map_err(Error::Transport),
        AddressSpace::Coil => t
            .write_coil(off, regs.first().is_some_and(|r| *r != 0))
            .await
            .map_err(Error::Transport),
        _ => Err(Error::UnsupportedMapping("cannot write this address space")),
    }
}

/// The sentinel meaning for registers that decoded to [`Value::Unavailable`].
pub fn na_meaning(p: &PointDesc<'static>, regs: &[u16]) -> &'static str {
    let Ok((raw, bits)) = decode_raw(p, regs) else {
        return "";
    };
    p.na.iter()
        .find(|na| (na.raw as u64) & mask_for(bits) == raw)
        .map(|na| na.meaning)
        .unwrap_or("")
}

const WRONG_KIND: &str = "decoded value kind does not match the generated signature";

pub fn value_f64<E>(v: Value) -> Result<f64, Error<E>> {
    v.as_f64().ok_or(Error::UnsupportedMapping(WRONG_KIND))
}

pub fn value_bool<E>(v: Value) -> Result<bool, Error<E>> {
    match v {
        Value::Bool(b) => Ok(b),
        _ => Err(Error::UnsupportedMapping(WRONG_KIND)),
    }
}

pub fn value_u64<E>(v: Value) -> Result<u64, Error<E>> {
    match v {
        Value::U64(x) => Ok(x),
        _ => Err(Error::UnsupportedMapping(WRONG_KIND)),
    }
}

pub fn value_i64<E>(v: Value) -> Result<i64, Error<E>> {
    match v {
        Value::I64(x) => Ok(x),
        Value::U64(x) => i64::try_from(x).map_err(|_| Error::UnsupportedMapping(WRONG_KIND)),
        _ => Err(Error::UnsupportedMapping(WRONG_KIND)),
    }
}

/// Raw window of a FLAGS point (feed a generated flags struct).
pub fn value_flags<E>(v: Value) -> Result<u64, Error<E>> {
    match v {
        Value::Flags(m) => Ok(m),
        _ => Err(Error::UnsupportedMapping(WRONG_KIND)),
    }
}

/// Raw window of a packed-fields point (feed a generated field struct).
pub fn value_fields<E>(v: Value) -> Result<u64, Error<E>> {
    match v {
        Value::Fields(w) => Ok(w),
        _ => Err(Error::UnsupportedMapping(WRONG_KIND)),
    }
}

/// Epoch value per the point's datetime encoding.
pub fn value_datetime<E>(v: Value) -> Result<i64, Error<E>> {
    match v {
        Value::DateTime(t) => Ok(t),
        _ => Err(Error::UnsupportedMapping(WRONG_KIND)),
    }
}

/// Integer value of a scale_ref / selector_ref companion point.
pub fn ref_int<E>(v: Value) -> Result<i64, Error<E>> {
    v.as_i64().ok_or(Error::UnsupportedMapping(WRONG_KIND))
}

/// "SunS" marker as two big-endian 16-bit words.
const SUNS_MARKER: [u16; 2] = [0x5375, 0x6e53];

/// Probe SunSpec anchors, walk the (model_id, length) chain, and return the
/// offset of the target model's ID register (spec §7.3; generated discovery
/// blocks cache the result).
pub async fn resolve_sunspec<T: Transport>(
    t: &mut T,
    space: AddressSpace,
    anchors: &[u16],
    model_id: u16,
) -> Result<u16, Error<T::Error>> {
    async fn header<T: Transport>(
        t: &mut T,
        space: AddressSpace,
        off: u16,
    ) -> Result<[u16; 2], T::Error> {
        let mut hdr = [0u16; 2];
        match space {
            AddressSpace::InputRegister => t.read_input(off, &mut hdr).await?,
            _ => t.read_holding(off, &mut hdr).await?,
        }
        Ok(hdr)
    }

    let mut anchor = None;
    for &a in anchors {
        // Devices answer exceptions off-anchor; try the next candidate.
        if let Ok(hdr) = header(t, space, a).await {
            if hdr == SUNS_MARKER {
                anchor = Some(a);
                break;
            }
        }
    }
    let Some(a) = anchor else {
        return Err(Error::UnsupportedMapping("SunS marker not found"));
    };

    let mut off = a + 2;
    for _ in 0..256 {
        let hdr = header(t, space, off).await.map_err(Error::Transport)?;
        if hdr[0] == 0xffff {
            break;
        }
        if hdr[0] == model_id {
            return Ok(off);
        }
        off = off
            .checked_add(2 + hdr[1])
            .ok_or(Error::UnsupportedMapping("SunSpec model chain overflows"))?;
    }
    Err(Error::UnsupportedMapping("SunSpec model not found"))
}
