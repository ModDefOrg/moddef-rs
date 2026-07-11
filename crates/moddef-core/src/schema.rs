//! Generated protobuf schema types (spec §27), compiled by build.rs from the
//! vendored proto/moddef/v1/*.proto via protox + prost-build. Under `std`,
//! pbjson-generated serde impls give protojson-semantics JSON (and, via
//! `document`, YAML) for every message.
//!
//! Maps use BTreeMap (`btree_map=.`), which keeps the types `alloc`-only and
//! binary encoding deterministic — conformance tests compare byte-for-byte
//! with Go-produced `.moddef` goldens.

#[allow(clippy::all, missing_docs)]
mod pb {
    include!(concat!(env!("OUT_DIR"), "/moddef.v1.rs"));

    #[cfg(feature = "std")]
    include!(concat!(env!("OUT_DIR"), "/moddef.v1.serde.rs"));
}

pub use pb::*;
