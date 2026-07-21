// SPDX-License-Identifier: Apache-2.0

//! prost [`schema::Point`] → [`PointDesc`] conversion (`alloc`). The codec
//! core is heap-free, so descriptors borrow: strings and `allowed_values`
//! borrow the prost message directly; flag/field/case/na tables need owned
//! side-buffers ([`DescBufs`]) because their layout differs from the wire
//! form. Two-step use:
//!
//! ```ignore
//! let bufs = desc_bufs(point);
//! let desc = point_desc(point, block.space(), &bufs);
//! ```
//!
//! Kind/precedence decisions mirror go/codec and moddef-ts `decodePoint`:
//! composed > flags > fields > string/bytes > primitive (default: raw enum
//! integer, signed if the storage is).

use alloc::vec::Vec;

use crate::desc::{
    Access, AddressSpace, ComposedSub, DateTimeEncoding, FieldDesc, NaDesc, PointDesc, Rational,
    ScaleMode, ScaleRefDesc, SelectorCaseDesc, SelectorDesc, StorageType, StringPadding,
    StringTermination, ValueKind, WriteDesc,
};
use crate::schema;

/// Owned side-tables backing one [`PointDesc`] view. Strings inside borrow
/// the prost message, so this lives exactly as long as the borrow chain.
#[derive(Debug, Default)]
pub struct DescBufs<'a> {
    flags: Vec<(u8, &'a str)>,
    fields: Vec<FieldDesc<'a>>,
    cases: Vec<SelectorCaseDesc>,
    na: Vec<NaDesc<'a>>,
}

/// Build the side-buffers for `p` (step 1 of 2).
pub fn desc_bufs(p: &schema::Point) -> DescBufs<'_> {
    let mut b = DescBufs::default();

    if let Some(schema::value_type::Kind::Flags(fl)) =
        p.value_type.as_ref().and_then(|v| v.kind.as_ref())
    {
        // BTreeMap iterates ascending — same order the TS decoder sorts into.
        for (bit, name) in &fl.bits {
            b.flags.push((*bit as u8, name.as_str()));
        }
    }

    // Legacy bit_fields first, then §13.1 fields (TS decodeFields order).
    for f in &p.bit_fields {
        b.fields.push(FieldDesc {
            id: &f.field_id,
            bit_offset: f.bit_offset,
            bit_length: f.bit_length,
        });
    }
    for f in &p.fields {
        b.fields.push(FieldDesc {
            id: &f.field_id,
            bit_offset: f.bit_offset,
            bit_length: f.bit_length,
        });
    }

    if let Some(sel) = &p.selector_ref {
        for (key, c) in &sel.cases {
            b.cases.push(SelectorCaseDesc {
                key: *key,
                scale: c.scale.as_ref().map(rational),
                offset: c.offset.as_ref().map(rational),
            });
        }
    }

    for na in &p.na_values {
        b.na.push(NaDesc {
            raw: na.raw,
            meaning: &na.meaning,
        });
    }

    b
}

/// Build the descriptor view for `p` (step 2 of 2). `block_space` is the
/// owning block's address space, used when the mapping leaves it unspecified.
pub fn point_desc<'a>(
    p: &'a schema::Point,
    block_space: schema::AddressSpace,
    bufs: &'a DescBufs<'a>,
) -> PointDesc<'a> {
    let m = p.mapping.as_ref();
    let t = p.transform.as_ref();

    let space = m
        .map(|m| m.space())
        .filter(|s| *s != schema::AddressSpace::Unspecified)
        .unwrap_or(block_space);

    let storage = storage_type(p.storage_type());

    PointDesc {
        id: &p.point_id,
        space: address_space(space),
        offset: m.map(|m| m.offset as u16).unwrap_or(0),
        model_relative_offset: m.map(|m| m.model_relative_offset as u16).unwrap_or(0),
        length_words: m.map(|m| m.length_words as u16).unwrap_or(0),
        storage,
        value: value_kind(p, storage, bufs),
        byte_big: m
            .map(|m| m.byte_order() != schema::ByteOrder::LittleEndian)
            .unwrap_or(true),
        word_big: m
            .map(|m| m.word_order() != schema::WordOrder::WordLittleEndian)
            .unwrap_or(true),
        scale: t.and_then(|t| t.scale.as_ref()).map(rational),
        offset_add: t.and_then(|t| t.offset.as_ref()).map(rational),
        scale_ref: t.and_then(|t| t.scale_ref.as_ref()).map(|sr| ScaleRefDesc {
            point_id: &sr.point_id,
            mode: if sr.mode() == schema::ScaleMode::Multiply {
                ScaleMode::Multiply
            } else {
                ScaleMode::Pow10
            },
            denominator: sr.denominator,
        }),
        selector: p.selector_ref.as_ref().map(|sel| SelectorDesc {
            point_id: &sel.point_id,
            cases: &bufs.cases,
        }),
        na: &bufs.na,
        access: match p.access() {
            schema::AccessMode::WriteOnly => Access::WriteOnly,
            schema::AccessMode::ReadWrite => Access::ReadWrite,
            schema::AccessMode::Command => Access::Command,
            _ => Access::ReadOnly,
        },
        write: p
            .write
            .as_ref()
            .and_then(|w| w.constraints.as_ref())
            .map(|c| WriteDesc {
                min: c.min_value.as_ref().map(rational),
                max: c.max_value.as_ref().map(rational),
                step: c.step.as_ref().map(rational),
                allowed: &c.allowed_values,
            }),
    }
}

/// Register count to read/write for `p` (mapping length or storage default).
pub fn point_words(p: &schema::Point) -> usize {
    let lw = p.mapping.as_ref().map(|m| m.length_words).unwrap_or(0);
    if lw > 0 {
        lw as usize
    } else {
        storage_type(p.storage_type()).default_words()
    }
}

fn rational(r: &schema::Rational) -> Rational {
    Rational {
        num: r.numerator,
        den: r.denominator,
    }
}

fn address_space(s: schema::AddressSpace) -> AddressSpace {
    match s {
        schema::AddressSpace::Coil => AddressSpace::Coil,
        schema::AddressSpace::DiscreteInput => AddressSpace::DiscreteInput,
        schema::AddressSpace::InputRegister => AddressSpace::InputRegister,
        _ => AddressSpace::HoldingRegister,
    }
}

fn storage_type(s: schema::StorageType) -> StorageType {
    match s {
        schema::StorageType::Bit => StorageType::Bit,
        schema::StorageType::U16 => StorageType::U16,
        schema::StorageType::S16 => StorageType::S16,
        schema::StorageType::U24 => StorageType::U24,
        schema::StorageType::U32 => StorageType::U32,
        schema::StorageType::S32 => StorageType::S32,
        schema::StorageType::U48 => StorageType::U48,
        schema::StorageType::S48 => StorageType::S48,
        schema::StorageType::U64 => StorageType::U64,
        schema::StorageType::S64 => StorageType::S64,
        schema::StorageType::Ieee754F32 => StorageType::F32,
        schema::StorageType::Ieee754F64 => StorageType::F64,
        schema::StorageType::StringAscii => StorageType::StringAscii,
        schema::StorageType::StringUtf8 => StorageType::StringUtf8,
        schema::StorageType::BytesRaw => StorageType::BytesRaw,
        schema::StorageType::Bcd => StorageType::Bcd,
        schema::StorageType::Composed => StorageType::Composed,
        // FIXED_POINT and unspecified: width from the mapping, unsigned —
        // the Go/TS default branch.
        _ => StorageType::Unspecified,
    }
}

fn value_kind<'a>(
    p: &'a schema::Point,
    storage: StorageType,
    bufs: &'a DescBufs<'a>,
) -> ValueKind<'a> {
    // §14 composed first: from the composed sub-mappings. A missing/zero base
    // is caught at decode time (ComposedBaseZero), matching Go/TS.
    if storage == StorageType::Composed || p.mapping.as_ref().is_some_and(|m| m.composed.is_some())
    {
        let c = p.mapping.as_ref().and_then(|m| m.composed.as_deref());
        let sub = |m: Option<&schema::Mapping>| -> ComposedSub {
            let Some(m) = m else {
                return ComposedSub {
                    offset: 0,
                    words: 1,
                    bit_offset: 0,
                    bit_length: 0,
                    width_bits: 16,
                    signed: true,
                };
            };
            let words = if m.length_words == 0 {
                1
            } else {
                m.length_words as u8
            };
            // The sub-mapping's storage_type supplies signedness and width;
            // absent one the sub-value is signed over its whole window
            // (pre-v0.5 behavior).
            let st = storage_type(m.storage_type());
            ComposedSub {
                offset: m.offset as u16,
                words,
                bit_offset: m.bit_offset as u8,
                bit_length: m.bit_length as u8,
                width_bits: st.bits(words as usize) as u8,
                signed: st == StorageType::Unspecified || st.signed(),
            }
        };
        return ValueKind::Composed {
            base: c.map(|c| c.base).unwrap_or(0),
            mantissa: sub(c.and_then(|c| c.mantissa.as_deref())),
            exponent: sub(c.and_then(|c| c.exponent.as_deref())),
        };
    }

    if !bufs.flags.is_empty()
        || matches!(
            p.value_type.as_ref().and_then(|v| v.kind.as_ref()),
            Some(schema::value_type::Kind::Flags(_))
        )
    {
        return ValueKind::Flags(&bufs.flags);
    }

    if !bufs.fields.is_empty() {
        return ValueKind::Fields(&bufs.fields);
    }

    if storage == StorageType::StringAscii || storage == StorageType::StringUtf8 {
        let enc = p.mapping.as_ref().and_then(|m| m.string_encoding.as_ref());
        return ValueKind::Str {
            padding: match enc.map(|e| e.padding()) {
                Some(schema::Padding::Null) => StringPadding::Null,
                Some(schema::Padding::Space) => StringPadding::Space,
                _ => StringPadding::None,
            },
            termination: match enc.map(|e| e.termination()) {
                Some(schema::Termination::NullTerminated) => StringTermination::NullTerminated,
                _ => StringTermination::FixedLength,
            },
        };
    }
    if storage == StorageType::BytesRaw {
        return ValueKind::Bytes;
    }

    let prim = match p.value_type.as_ref().and_then(|v| v.kind.as_ref()) {
        Some(schema::value_type::Kind::Primitive(v)) => {
            schema::PrimitiveType::try_from(*v).unwrap_or(schema::PrimitiveType::Unspecified)
        }
        _ => schema::PrimitiveType::Unspecified,
    };

    match prim {
        schema::PrimitiveType::Bool => ValueKind::Bool,
        schema::PrimitiveType::Datetime => ValueKind::DateTime(
            if p.datetime.as_ref().map(|d| d.encoding()) == Some(schema::DateTimeEncoding::EpochMs)
            {
                DateTimeEncoding::EpochMillis
            } else {
                // EPOCH_S and unspecified — the Go/TS default.
                DateTimeEncoding::EpochSeconds
            },
        ),
        schema::PrimitiveType::Decimal
        | schema::PrimitiveType::Float32
        | schema::PrimitiveType::Float64 => ValueKind::Decimal,
        schema::PrimitiveType::Uint32 | schema::PrimitiveType::Uint64 => ValueKind::Uint,
        schema::PrimitiveType::Int32 | schema::PrimitiveType::Int64 => ValueKind::Int,
        // enum_ref / struct_ref / no primitive: raw integer via the enum path.
        _ => ValueKind::Enum,
    }
}
