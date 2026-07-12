// SPDX-License-Identifier: Apache-2.0

//! ModDef runtime for Rust (spec v0.4).
//!
//! Layering (spec §32):
//! - **codec core** (always available, `no_std`, no alloc): [`desc::PointDesc`]
//!   descriptors + [`codec`] decode/encode + [`Transport`] trait + errors.
//! - **`alloc`**: prost schema types ([`schema`]), owned [`value::DecodedValue`],
//!   the untyped [`device::Device`] facade, measurand queries, binary `.moddef`
//!   parsing.
//! - **`std`** (default): protojson YAML/JSON parsing ([`document`]), fs
//!   helpers, `std::error::Error` impls.
//!
//! Embedded targets use generated `static` descriptor tables (see
//! `moddef-codegen`) and never parse documents at runtime; servers/gateways
//! parse `.moddef` files at runtime and use the dynamic facade. Both paths
//! share this codec.
#![cfg_attr(not(feature = "std"), no_std)]
#![deny(unsafe_code)]

#[cfg(feature = "alloc")]
extern crate alloc;

pub mod codec;
pub mod desc;
pub mod error;
pub mod rt;
pub mod transport;
pub mod value;

#[cfg(feature = "alloc")]
pub mod schema;

#[cfg(feature = "alloc")]
pub mod convert;

#[cfg(feature = "alloc")]
pub mod device;

#[cfg(feature = "alloc")]
pub mod measurand;

#[cfg(feature = "std")]
pub mod document;

#[cfg(feature = "std")]
pub mod resolve;

pub use desc::{
    Access, AddressSpace, DateTimeEncoding, FieldDesc, NaDesc, PointDesc, Rational, ScaleMode,
    ScaleRefDesc, SelectorCaseDesc, SelectorDesc, StorageType, StringPadding, StringTermination,
    ValueKind, WriteDesc,
};
pub use error::{ConstraintKind, DecodeError, EncodeError, Error};
pub use transport::Transport;
pub use value::{Reading, Value};

#[cfg(feature = "alloc")]
pub use device::Device;
#[cfg(feature = "alloc")]
pub use measurand::MeasurandQuery;
#[cfg(feature = "alloc")]
pub use value::DecodedValue;

#[cfg(feature = "std")]
pub use document::{
    detect_format, load, parse_document, serialize_document, DocumentFormat, ParseError,
};
