//! Decoded value model (spec §8, §13). The no_std core surfaces [`Value`]
//! (no heap: strings decode into caller buffers, flags stay a raw mask with
//! name iteration via the descriptor); the `alloc` feature adds the owned
//! [`DecodedValue`] mirroring the Go/TS union.

use crate::desc::{FieldDesc, PointDesc, ValueKind};

/// A reading that may be a device-reported "no data" sentinel (§8.4).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Reading<T> {
    Value(T),
    Unavailable(&'static str),
}

impl<T> Reading<T> {
    pub fn value(self) -> Option<T> {
        match self {
            Reading::Value(v) => Some(v),
            Reading::Unavailable(_) => None,
        }
    }
}

/// Heap-free decoded value.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Value {
    Bool(bool),
    U64(u64),
    I64(i64),
    F64(f64),
    /// §13.2 flag set: raw mask; iterate names with [`flag_names`].
    Flags(u64),
    /// §13 packed sub-fields: raw window; extract with [`field_value`].
    Fields(u64),
    /// §8.5 epoch value (seconds or millis per the point's encoding).
    DateTime(i64),
    /// §8.4 sentinel hit. The meaning str borrows the descriptor table.
    Unavailable,
}

impl Value {
    pub fn as_f64(self) -> Option<f64> {
        match self {
            Value::F64(v) => Some(v),
            Value::U64(v) => Some(v as f64),
            Value::I64(v) => Some(v as f64),
            Value::Bool(b) => Some(if b { 1.0 } else { 0.0 }),
            _ => None,
        }
    }

    pub fn as_i64(self) -> Option<i64> {
        match self {
            Value::I64(v) => Some(v),
            Value::U64(v) => i64::try_from(v).ok(),
            Value::Bool(b) => Some(b as i64),
            Value::F64(v) => Some(v as i64),
            Value::Flags(m) => i64::try_from(m).ok(),
            Value::Fields(w) => i64::try_from(w).ok(),
            Value::DateTime(t) => Some(t),
            Value::Unavailable => None,
        }
    }
}

impl From<f64> for Value {
    fn from(v: f64) -> Self {
        Value::F64(v)
    }
}

impl From<i64> for Value {
    fn from(v: i64) -> Self {
        Value::I64(v)
    }
}

impl From<u64> for Value {
    fn from(v: u64) -> Self {
        Value::U64(v)
    }
}

impl From<bool> for Value {
    fn from(v: bool) -> Self {
        Value::Bool(v)
    }
}

/// Iterate the names of set flags for a FLAGS point (ascending bit order).
pub fn flag_names<'a>(desc: &PointDesc<'a>, mask: u64) -> impl Iterator<Item = &'a str> {
    let table: &'a [(u8, &'a str)] = match desc.value {
        ValueKind::Flags(t) => t,
        _ => &[],
    };
    table
        .iter()
        .filter(move |(bit, _)| mask & (1u64 << bit) != 0)
        .map(|(_, name)| *name)
}

/// Extract one sub-field from a packed window raw value (§13).
pub fn field_value(f: &FieldDesc<'_>, window: u64) -> u64 {
    let mask = if f.bit_length >= 64 {
        u64::MAX
    } else {
        (1u64 << f.bit_length) - 1
    };
    (window >> f.bit_offset) & mask
}

/// Owned decoded value (parity with the Go/TS union), `alloc` only.
#[cfg(feature = "alloc")]
#[derive(Clone, Debug, PartialEq)]
pub enum DecodedValue {
    Bool(bool),
    U64(u64),
    I64(i64),
    F64(f64),
    Str(alloc::string::String),
    Bytes(alloc::vec::Vec<u8>),
    /// Names of set flags.
    Flags(alloc::vec::Vec<alloc::string::String>),
    /// field_id -> numeric sub-value.
    Fields(alloc::vec::Vec<(alloc::string::String, u64)>),
    DateTime(i64),
    Unavailable(alloc::string::String),
}

#[cfg(feature = "alloc")]
impl DecodedValue {
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            DecodedValue::F64(v) => Some(*v),
            DecodedValue::U64(v) => Some(*v as f64),
            DecodedValue::I64(v) => Some(*v as f64),
            DecodedValue::Bool(b) => Some(if *b { 1.0 } else { 0.0 }),
            _ => None,
        }
    }

    pub fn as_i64(&self) -> Option<i64> {
        match self {
            DecodedValue::I64(v) => Some(*v),
            DecodedValue::U64(v) => i64::try_from(*v).ok(),
            DecodedValue::Bool(b) => Some(*b as i64),
            DecodedValue::F64(v) => Some(*v as i64),
            DecodedValue::DateTime(t) => Some(*t),
            _ => None,
        }
    }

    pub fn is_unavailable(&self) -> bool {
        matches!(self, DecodedValue::Unavailable(_))
    }
}
