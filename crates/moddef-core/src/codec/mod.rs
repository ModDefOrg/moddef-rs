// SPDX-License-Identifier: Apache-2.0

//! Codec core (spec §8–§15): pure functions over [`crate::desc::PointDesc`],
//! `no_std` and allocation-free. Kept in behavioral lockstep with go/codec
//! and moddef-ts's codec (shared vector suite in the conformance tests).

mod bytes;
mod rat;

pub mod decode;
pub mod encode;

pub use bytes::{assemble_u64, mask_for, words_from_u64};
pub use decode::{decode, decode_bytes, decode_raw, decode_str, Ctx};
pub use encode::{encode, encode_bytes, encode_str, validate_write};
pub use rat::Rat;
