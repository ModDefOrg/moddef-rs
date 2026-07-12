// SPDX-License-Identifier: Apache-2.0

//! Typed Rust client generator for ModDef documents (spec §31).
//!
//! `generate(&ModDefDocument)` emits one deterministic, `no_std`-compatible
//! module per document; the `moddef-rs` binary wraps it as
//! `moddef-rs gen -o <dir> <file.moddef.yaml>...`.

mod generate;
mod naming;

pub use generate::{generate, GeneratedFile};
