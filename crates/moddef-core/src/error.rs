//! Typed errors (spec §26.3/§26.4, §32). Structured variants rather than
//! strings; `Display` always, `std::error::Error` under `std`.

use core::fmt;

/// Codec decode failure causes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DecodeError {
    /// scale_ref / selector_ref target not resolved in the context.
    UnresolvedRef,
    ZeroScaleDenominator,
    ComposedBaseZero,
    /// Register slice shorter than the point's width.
    ShortRead,
    /// Output buffer too small (string decode).
    BufferTooSmall,
    InvalidUtf8,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EncodeError {
    NotWritable,
    UnresolvedRef,
    /// Composed / packed-field windows are read-oriented (§14, §13.1).
    Unsupported,
    WrongValueType,
    BufferTooSmall,
}

/// §11.4 constraint that a write value violated.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConstraintKind {
    Min,
    Max,
    Step,
    AllowedValues,
}

/// Facade error, generic over the transport's error type.
#[derive(Debug)]
pub enum Error<T> {
    Transport(T),
    DeviceNotFound,
    PointNotFound,
    MeasurandNotSupported,
    /// More than one point matches the measurand query (spec §26.4).
    AmbiguousMeasurand,
    /// Composed points via facade, unknown discovery kind, SunS not found…
    UnsupportedMapping(&'static str),
    Decode(DecodeError),
    Encode(EncodeError),
    WriteAccess,
    WriteConstraint(ConstraintKind),
}

impl<T> From<DecodeError> for Error<T> {
    fn from(e: DecodeError) -> Self {
        Error::Decode(e)
    }
}

impl<T> From<EncodeError> for Error<T> {
    fn from(e: EncodeError) -> Self {
        Error::Encode(e)
    }
}

impl fmt::Display for DecodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DecodeError::UnresolvedRef => write!(f, "scale/selector ref not resolved in context"),
            DecodeError::ZeroScaleDenominator => write!(f, "scale denominator is zero"),
            DecodeError::ComposedBaseZero => write!(f, "composed base is zero"),
            DecodeError::ShortRead => write!(f, "register window shorter than point width"),
            DecodeError::BufferTooSmall => write!(f, "output buffer too small"),
            DecodeError::InvalidUtf8 => write!(f, "decoded string is not valid UTF-8"),
        }
    }
}

impl fmt::Display for EncodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EncodeError::NotWritable => write!(f, "point is not writable"),
            EncodeError::UnresolvedRef => write!(f, "scale ref not resolved in context"),
            EncodeError::Unsupported => write!(f, "value kind is not encodable"),
            EncodeError::WrongValueType => write!(f, "value type does not match the point"),
            EncodeError::BufferTooSmall => write!(f, "register buffer too small"),
        }
    }
}

impl fmt::Display for ConstraintKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ConstraintKind::Min => write!(f, "min_value"),
            ConstraintKind::Max => write!(f, "max_value"),
            ConstraintKind::Step => write!(f, "step"),
            ConstraintKind::AllowedValues => write!(f, "allowed_values"),
        }
    }
}

impl<T: fmt::Display> fmt::Display for Error<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Transport(e) => write!(f, "transport: {e}"),
            Error::DeviceNotFound => write!(f, "device profile not found"),
            Error::PointNotFound => write!(f, "point not found"),
            Error::MeasurandNotSupported => write!(f, "measurand not supported"),
            Error::AmbiguousMeasurand => write!(f, "measurand query is ambiguous"),
            Error::UnsupportedMapping(d) => write!(f, "unsupported mapping: {d}"),
            Error::Decode(e) => write!(f, "decode: {e}"),
            Error::Encode(e) => write!(f, "encode: {e}"),
            Error::WriteAccess => write!(f, "point is not writable"),
            Error::WriteConstraint(k) => write!(f, "write violates constraint {k}"),
        }
    }
}

#[cfg(feature = "std")]
impl std::error::Error for DecodeError {}
#[cfg(feature = "std")]
impl std::error::Error for EncodeError {}
#[cfg(feature = "std")]
impl<T: fmt::Display + fmt::Debug> std::error::Error for Error<T> {}
