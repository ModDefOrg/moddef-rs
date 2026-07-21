// SPDX-License-Identifier: Apache-2.0

//! Lightweight, heap-free point descriptors — the codec core's view of a
//! point (spec §7–§15). Generated code emits `static` tables of these;
//! the `alloc` feature converts prost `Point`s into them (see `convert`).
//!
//! Lifetimes: `'a` is the backing storage — `'static` for generated tables,
//! the document's lifetime for runtime-parsed profiles.

/// §7 Modbus address space.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AddressSpace {
    Coil,
    DiscreteInput,
    InputRegister,
    HoldingRegister,
}

/// §8.2 Physical storage type.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StorageType {
    /// No storage type declared: width comes from the mapping's length_words,
    /// unsigned (matches the Go/TS default branch).
    Unspecified,
    Bit,
    U16,
    S16,
    U24,
    U32,
    S32,
    U48,
    S48,
    U64,
    S64,
    F32,
    F64,
    StringAscii,
    StringUtf8,
    BytesRaw,
    Bcd,
    Composed,
}

impl StorageType {
    pub const fn bits(self, words: usize) -> u32 {
        match self {
            StorageType::Bit => 1,
            StorageType::U16 | StorageType::S16 => 16,
            StorageType::U24 => 24,
            StorageType::U32 | StorageType::S32 => 32,
            StorageType::U48 | StorageType::S48 => 48,
            StorageType::U64 | StorageType::S64 => 64,
            _ => {
                let w = if words == 0 { 1 } else { words };
                if w * 16 > 64 {
                    64
                } else {
                    (w * 16) as u32
                }
            }
        }
    }

    pub const fn signed(self) -> bool {
        matches!(
            self,
            StorageType::S16 | StorageType::S32 | StorageType::S48 | StorageType::S64
        )
    }

    /// Default register width when the mapping omits length_words.
    pub const fn default_words(self) -> usize {
        match self {
            StorageType::U32 | StorageType::S32 | StorageType::F32 | StorageType::U24 => 2,
            StorageType::U48 | StorageType::S48 => 3,
            StorageType::U64 | StorageType::S64 | StorageType::F64 => 4,
            _ => 1,
        }
    }
}

/// §11 Access mode.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Access {
    ReadOnly,
    WriteOnly,
    ReadWrite,
    Command,
}

/// §10.1 Rational scale/offset.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rational {
    pub num: i64,
    pub den: i64,
}

/// §10.4 Register-referenced scaling mode.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScaleMode {
    Pow10,
    Multiply,
}

#[derive(Clone, Copy, Debug)]
pub struct ScaleRefDesc<'a> {
    pub point_id: &'a str,
    pub mode: ScaleMode,
    /// MULTIPLY denominator (0 treated as 1).
    pub denominator: i64,
}

/// §10.5 One selector case: scale/offset chosen by the selector value.
#[derive(Clone, Copy, Debug)]
pub struct SelectorCaseDesc {
    pub key: i64,
    pub scale: Option<Rational>,
    pub offset: Option<Rational>,
}

#[derive(Clone, Copy, Debug)]
pub struct SelectorDesc<'a> {
    pub point_id: &'a str,
    pub cases: &'a [SelectorCaseDesc],
}

/// §8.4 Unavailable/sentinel raw value.
#[derive(Clone, Copy, Debug)]
pub struct NaDesc<'a> {
    pub raw: i64,
    pub meaning: &'a str,
}

/// §13 Bit/register sub-field.
#[derive(Clone, Copy, Debug)]
pub struct FieldDesc<'a> {
    pub id: &'a str,
    pub bit_offset: u32,
    pub bit_length: u32,
}

/// §8.5 Date/time encoding.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DateTimeEncoding {
    EpochSeconds,
    EpochMillis,
}

/// §15 String padding / termination.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StringPadding {
    None,
    Null,
    Space,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StringTermination {
    FixedLength,
    NullTerminated,
}

/// §8/§13 Logical value interpretation.
#[derive(Clone, Copy, Debug)]
pub enum ValueKind<'a> {
    /// Scaled numeric (§10) — DECIMAL / FLOAT primitives.
    Decimal,
    Bool,
    /// UINT32/UINT64 raw.
    Uint,
    /// INT32/INT64 raw.
    Int,
    /// Enum-backed: raw integer, mapped by the caller / generated enum.
    Enum,
    /// §13.2 flag set: (bit, name).
    Flags(&'a [(u8, &'a str)]),
    /// §13/§13.1 packed sub-fields.
    Fields(&'a [FieldDesc<'a>]),
    Str {
        padding: StringPadding,
        termination: StringTermination,
    },
    Bytes,
    DateTime(DateTimeEncoding),
    /// §14 mantissa/exponent over the read window.
    Composed {
        base: i64,
        mantissa: ComposedSub,
        exponent: ComposedSub,
    },
}

/// §14 composed sub-mapping (word offset relative to the read window),
/// optionally carrying a §14.2 bit window — the embedded decade exponent,
/// where mantissa and exponent share a word (Iskra T5/T6, Eaton PXM).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ComposedSub {
    pub offset: u16,
    pub words: u8,
    /// Bit window over the assembled sub-window (LSB = 0).
    pub bit_offset: u8,
    /// Bit window length; 0 = no bit window (whole sub-window).
    pub bit_length: u8,
    /// Width in bits when no bit window applies (storage or window width).
    pub width_bits: u8,
    /// Sign-extend — from bit_length when windowed, else width_bits. True
    /// when the sub-mapping declares no storage_type (pre-v0.5 behavior).
    pub signed: bool,
}

/// §11.4 Write constraints (engineering units).
#[derive(Clone, Copy, Debug, Default)]
pub struct WriteDesc<'a> {
    pub min: Option<Rational>,
    pub max: Option<Rational>,
    pub step: Option<Rational>,
    pub allowed: &'a [i64],
}

/// The codec core's complete view of one point.
#[derive(Clone, Copy, Debug)]
pub struct PointDesc<'a> {
    pub id: &'a str,
    pub space: AddressSpace,
    pub offset: u16,
    /// §7.3 SunSpec model-relative offset (ID register = 0). Used when the
    /// owning block declares discovery.
    pub model_relative_offset: u16,
    pub length_words: u16,
    pub storage: StorageType,
    pub value: ValueKind<'a>,
    /// Byte order within a word: big-endian unless false (§9.1).
    pub byte_big: bool,
    /// Word order across words: big-endian unless false (§9.2).
    pub word_big: bool,
    pub scale: Option<Rational>,
    pub offset_add: Option<Rational>,
    pub scale_ref: Option<ScaleRefDesc<'a>>,
    pub selector: Option<SelectorDesc<'a>>,
    pub na: &'a [NaDesc<'a>],
    pub access: Access,
    pub write: Option<WriteDesc<'a>>,
}

impl<'a> PointDesc<'a> {
    /// Register count to read/write for this point.
    pub const fn words(&self) -> usize {
        if self.length_words > 0 {
            self.length_words as usize
        } else {
            self.storage.default_words()
        }
    }

    pub const fn readable(&self) -> bool {
        matches!(self.access, Access::ReadOnly | Access::ReadWrite)
    }

    pub const fn writable(&self) -> bool {
        matches!(
            self.access,
            Access::ReadWrite | Access::WriteOnly | Access::Command
        )
    }
}

/// A minimal defaulted descriptor for building tables/tests concisely.
pub const fn point(
    id: &str,
    space: AddressSpace,
    offset: u16,
    storage: StorageType,
) -> PointDesc<'_> {
    PointDesc {
        id,
        space,
        offset,
        model_relative_offset: 0,
        length_words: 0,
        storage,
        value: ValueKind::Decimal,
        byte_big: true,
        word_big: true,
        scale: None,
        offset_add: None,
        scale_ref: None,
        selector: None,
        na: &[],
        access: Access::ReadOnly,
        write: None,
    }
}
