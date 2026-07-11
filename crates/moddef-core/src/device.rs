//! Runtime device facade (spec §32.4): binds a [`Transport`] to one parsed
//! device profile for point- and measurand-based reads/writes, with no
//! codegen. Port of moddef-ts `Device` / go/client/client.go.
//!
//! SunSpec `model_relative_offset` is resolved against the model *ID
//! register* (offset 0 = model id, 1 = length, data at 2+) per spec §7.3 —
//! the same convention as the Go/TS clients and the profiles in devices/.
//!
//! Shared limitation kept in lockstep with Go/TS: composed (multi-register
//! mantissa/exponent) points decode via the codec directly, not through the
//! facade.

use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use crate::codec::decode::{decode, decode_bytes, decode_raw, decode_str, Ctx};
use crate::codec::encode::{encode, encode_str, validate_write};
use crate::codec::mask_for;
use crate::convert::{desc_bufs, point_desc, point_words};
use crate::desc::{DateTimeEncoding, ValueKind};
use crate::error::{DecodeError, Error};
use crate::measurand::{measurand_matches, MeasurandQuery};
use crate::schema;
use crate::transport::Transport;
use crate::value::{field_value, flag_names, DecodedValue, Value};

/// "SunS" marker as two big-endian 16-bit words.
const SUNS_MARKER: [u16; 2] = [0x5375, 0x6e53];

/// The untyped runtime facade over one device profile. Generated typed
/// clients (moddef-codegen) skip this and use `static` descriptor tables.
pub struct Device<'d, T: Transport> {
    profile: &'d schema::DeviceProfile,
    transport: T,
    /// Resolved SunSpec model ID-register offsets, cached per block (§7.3).
    model_base: BTreeMap<&'d str, u16>,
}

impl<'d, T: Transport> Device<'d, T> {
    /// Bind a transport to the named device profile in `doc` (or the only one).
    pub fn new(
        doc: &'d schema::ModDefDocument,
        device_id: Option<&str>,
        transport: T,
    ) -> Result<Self, Error<T::Error>> {
        let profile = doc
            .devices
            .iter()
            .find(|d| device_id.is_none_or(|id| d.device_id == id))
            .ok_or(Error::DeviceNotFound)?;
        Ok(Device::from_profile(profile, transport))
    }

    pub fn from_profile(profile: &'d schema::DeviceProfile, transport: T) -> Self {
        Device {
            profile,
            transport,
            model_base: BTreeMap::new(),
        }
    }

    pub fn profile(&self) -> &'d schema::DeviceProfile {
        self.profile
    }

    pub fn transport_mut(&mut self) -> &mut T {
        &mut self.transport
    }

    pub fn into_transport(self) -> T {
        self.transport
    }

    /// All points in block order (spec §32.1).
    pub fn points(&self) -> impl Iterator<Item = &'d schema::Point> {
        self.profile.blocks.iter().flat_map(|b| b.points.iter())
    }

    /// Look up a point and its owning block by id.
    pub fn point(
        &self,
        id: &str,
    ) -> Result<(&'d schema::Point, &'d schema::RegisterBlock), Error<T::Error>> {
        for b in &self.profile.blocks {
            if let Some(p) = b.points.iter().find(|p| p.point_id == id) {
                return Ok((p, b));
            }
        }
        Err(Error::PointNotFound)
    }

    /// Read and decode a single point by id.
    pub async fn read_point(&mut self, id: &str) -> Result<DecodedValue, Error<T::Error>> {
        let (p, b) = self.point(id)?;
        let regs = self.read_registers(p, b).await?;
        let refs = self.ref_context(p).await?;
        decode_owned(p, b.space(), &regs, &Ctx { refs: &refs }).map_err(Error::Decode)
    }

    /// Read a point by its semantic measurand tuple (spec §26.1).
    pub async fn read_measurand(
        &mut self,
        q: &MeasurandQuery<'_>,
    ) -> Result<DecodedValue, Error<T::Error>> {
        let mut found: Option<&'d schema::Point> = None;
        for p in self.points() {
            if measurand_matches(p.measurand.as_ref(), q) {
                if found.is_some() {
                    return Err(Error::AmbiguousMeasurand);
                }
                found = Some(p);
            }
        }
        let p = found.ok_or(Error::MeasurandNotSupported)?;
        self.read_point(&p.point_id).await
    }

    /// Encode and write a value, validating access mode and §11.4 constraints.
    pub async fn write_point(&mut self, id: &str, v: Value) -> Result<(), Error<T::Error>> {
        let (p, b) = self.point(id)?;
        let bufs = desc_bufs(p);
        let d = point_desc(p, b.space(), &bufs);
        if !d.writable() {
            return Err(Error::WriteAccess);
        }
        validate_write(&d, &v).map_err(Error::WriteConstraint)?;

        let space = effective_space(p, b);
        let off = self.offset_of(p, b).await?;

        if space == schema::AddressSpace::Coil {
            let on = v.as_f64().map(|f| f != 0.0).unwrap_or(false);
            return self
                .transport
                .write_coil(off, on)
                .await
                .map_err(Error::Transport);
        }
        if space != schema::AddressSpace::HoldingRegister {
            return Err(Error::UnsupportedMapping("cannot write this address space"));
        }
        let refs = self.ref_context(p).await?;
        let mut regs = vec![0u16; d.words()];
        encode(&d, &v, &Ctx { refs: &refs }, &mut regs)?;
        self.transport
            .write_holding(off, &regs)
            .await
            .map_err(Error::Transport)
    }

    /// Write a string point (STRING_ASCII / STRING_UTF8), §15.
    pub async fn write_point_str(&mut self, id: &str, s: &str) -> Result<(), Error<T::Error>> {
        let (p, b) = self.point(id)?;
        let bufs = desc_bufs(p);
        let d = point_desc(p, b.space(), &bufs);
        if !d.writable() {
            return Err(Error::WriteAccess);
        }
        if effective_space(p, b) != schema::AddressSpace::HoldingRegister {
            return Err(Error::UnsupportedMapping("cannot write this address space"));
        }
        let off = self.offset_of(p, b).await?;
        let mut regs = vec![0u16; d.words()];
        encode_str(&d, s, &mut regs)?;
        self.transport
            .write_holding(off, &regs)
            .await
            .map_err(Error::Transport)
    }

    // --- internals -------------------------------------------------------- //

    /// Read the points referenced by p's scale_ref / selector_ref
    /// (spec §10.4/§10.5), decoded to integers for the codec context.
    async fn ref_context(
        &mut self,
        p: &'d schema::Point,
    ) -> Result<Vec<(&'d str, i64)>, Error<T::Error>> {
        let mut ids: Vec<&'d str> = Vec::new();
        if let Some(sr) = p.transform.as_ref().and_then(|t| t.scale_ref.as_ref()) {
            ids.push(&sr.point_id);
        }
        if let Some(sel) = p.selector_ref.as_ref() {
            ids.push(&sel.point_id);
        }
        let mut refs = Vec::with_capacity(ids.len());
        for id in ids {
            let (rp, rb) = self.point(id)?;
            let regs = self.read_registers(rp, rb).await?;
            let bufs = desc_bufs(rp);
            let d = point_desc(rp, rb.space(), &bufs);
            let v = decode(&d, &regs, &Ctx::EMPTY)?;
            if let Some(iv) = v.as_i64() {
                refs.push((id, iv));
            }
        }
        Ok(refs)
    }

    async fn read_registers(
        &mut self,
        p: &'d schema::Point,
        b: &'d schema::RegisterBlock,
    ) -> Result<Vec<u16>, Error<T::Error>> {
        if p.storage_type() == schema::StorageType::Composed {
            return Err(Error::UnsupportedMapping(
                "composed points are not read via the facade",
            ));
        }
        let space = effective_space(p, b);
        let n = point_words(p);
        let off = self.offset_of(p, b).await?;
        self.read_space(space, off, n).await
    }

    async fn read_space(
        &mut self,
        space: schema::AddressSpace,
        off: u16,
        n: usize,
    ) -> Result<Vec<u16>, Error<T::Error>> {
        match space {
            schema::AddressSpace::HoldingRegister => {
                let mut regs = vec![0u16; n];
                self.transport
                    .read_holding(off, &mut regs)
                    .await
                    .map_err(Error::Transport)?;
                Ok(regs)
            }
            schema::AddressSpace::InputRegister => {
                let mut regs = vec![0u16; n];
                self.transport
                    .read_input(off, &mut regs)
                    .await
                    .map_err(Error::Transport)?;
                Ok(regs)
            }
            schema::AddressSpace::Coil => {
                let mut bits = [false];
                self.transport
                    .read_coils(off, &mut bits)
                    .await
                    .map_err(Error::Transport)?;
                Ok(vec![bits[0] as u16])
            }
            schema::AddressSpace::DiscreteInput => {
                let mut bits = [false];
                self.transport
                    .read_discrete(off, &mut bits)
                    .await
                    .map_err(Error::Transport)?;
                Ok(vec![bits[0] as u16])
            }
            schema::AddressSpace::Unspecified => {
                Err(Error::UnsupportedMapping("unspecified address space"))
            }
        }
    }

    async fn offset_of(
        &mut self,
        p: &schema::Point,
        b: &'d schema::RegisterBlock,
    ) -> Result<u16, Error<T::Error>> {
        let m = p
            .mapping
            .as_ref()
            .ok_or(Error::UnsupportedMapping("point has no mapping"))?;
        if b.discovery.is_some() {
            let base = self.resolve_model_base(b).await?;
            Ok(base + m.model_relative_offset as u16)
        } else {
            Ok(m.offset as u16)
        }
    }

    /// Probe discovery anchors for the SunS marker, walk the (model_id,
    /// length) chain, and return the offset of the target model's ID register.
    async fn resolve_model_base(
        &mut self,
        b: &'d schema::RegisterBlock,
    ) -> Result<u16, Error<T::Error>> {
        if let Some(base) = self.model_base.get(b.block_id.as_str()) {
            return Ok(*base);
        }
        let disc = b
            .discovery
            .as_ref()
            .ok_or(Error::UnsupportedMapping("block has no discovery"))?;
        if disc.kind() != schema::DiscoveryKind::Sunspec {
            return Err(Error::UnsupportedMapping("unsupported discovery kind"));
        }
        let space = b.space();
        let defaults = [40000u32, 50000, 0];
        let candidates: &[u32] = if disc.anchor_candidates.is_empty() {
            &defaults
        } else {
            &disc.anchor_candidates
        };

        let mut anchor = None;
        for &c in candidates {
            // Devices answer exceptions off-anchor; try the next candidate.
            if let Ok(hdr) = self.read_space(space, c as u16, 2).await {
                if hdr[..2] == SUNS_MARKER {
                    anchor = Some(c as u16);
                    break;
                }
            }
        }
        let Some(anchor) = anchor else {
            return Err(Error::UnsupportedMapping("SunS marker not found"));
        };

        // Walk model headers starting just after the marker.
        let mut off = anchor + 2;
        for _ in 0..256 {
            let hdr = self.read_space(space, off, 2).await?;
            let (id, length) = (hdr[0], hdr[1]);
            if id == 0xffff {
                break;
            }
            if id as u32 == disc.model_id {
                // Base is the model ID register (model_relative_offset 0, §7.3).
                self.model_base.insert(&b.block_id, off);
                return Ok(off);
            }
            off = off
                .checked_add(2 + length)
                .ok_or(Error::UnsupportedMapping("SunSpec model chain overflows"))?;
        }
        Err(Error::UnsupportedMapping("SunSpec model not found"))
    }
}

/// The effective address space: the mapping's, else the owning block's.
fn effective_space(p: &schema::Point, b: &schema::RegisterBlock) -> schema::AddressSpace {
    match p.mapping.as_ref().map(|m| m.space()) {
        Some(s) if s != schema::AddressSpace::Unspecified => s,
        _ => b.space(),
    }
}

/// Decode a point's registers into an owned [`DecodedValue`]. Date/times are
/// normalized to epoch **milliseconds** (parity with the TS `Date` surface).
fn decode_owned(
    p: &schema::Point,
    block_space: schema::AddressSpace,
    regs: &[u16],
    ctx: &Ctx<'_>,
) -> Result<DecodedValue, DecodeError> {
    let bufs = desc_bufs(p);
    let d = point_desc(p, block_space, &bufs);

    match d.value {
        ValueKind::Str { .. } => {
            let mut buf = vec![0u8; regs.len() * 2];
            let s = decode_str(&d, regs, &mut buf)?;
            Ok(DecodedValue::Str(String::from(s)))
        }
        ValueKind::Bytes => {
            let mut buf = vec![0u8; regs.len() * 2];
            let bytes = decode_bytes(&d, regs, &mut buf)?;
            Ok(DecodedValue::Bytes(bytes.to_vec()))
        }
        _ => Ok(match decode(&d, regs, ctx)? {
            Value::Bool(v) => DecodedValue::Bool(v),
            Value::U64(v) => DecodedValue::U64(v),
            Value::I64(v) => DecodedValue::I64(v),
            Value::F64(v) => DecodedValue::F64(v),
            Value::Flags(mask) => {
                DecodedValue::Flags(flag_names(&d, mask).map(String::from).collect())
            }
            Value::Fields(window) => {
                let fields = match d.value {
                    ValueKind::Fields(fs) => fs,
                    _ => &[],
                };
                DecodedValue::Fields(
                    fields
                        .iter()
                        .map(|f| (String::from(f.id), field_value(f, window)))
                        .collect(),
                )
            }
            Value::DateTime(t) => DecodedValue::DateTime(match d.value {
                ValueKind::DateTime(DateTimeEncoding::EpochMillis) => t,
                _ => t.saturating_mul(1000),
            }),
            Value::Unavailable => {
                // Recover the sentinel's meaning for the owned value.
                let (raw, bits) = decode_raw(&d, regs)?;
                let meaning =
                    d.na.iter()
                        .find(|na| (na.raw as u64) & mask_for(bits) == raw)
                        .map(|na| na.meaning)
                        .unwrap_or("");
                DecodedValue::Unavailable(String::from(meaning))
            }
        }),
    }
}
