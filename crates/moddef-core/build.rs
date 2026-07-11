//! Compiles the vendored ModDef proto schema (proto/moddef/v1, synced from
//! ../moddef/proto — see sync-schema.sh) with protox (pure-Rust, no protoc)
//! into prost types plus pbjson protojson Serialize/Deserialize impls.
//! btree_map keeps the types no_std(+alloc)-compatible and the binary
//! encoding deterministic.

use std::path::PathBuf;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let files = [
        "proto/moddef/v1/types.proto",
        "proto/moddef/v1/measurand.proto",
        "proto/moddef/v1/mapping.proto",
        "proto/moddef/v1/device.proto",
        "proto/moddef/v1/document.proto",
    ];
    for f in files {
        println!("cargo:rerun-if-changed={f}");
    }

    let fds = protox::compile(files, ["proto"])?;

    let out = PathBuf::from(std::env::var("OUT_DIR")?);
    let descriptor_path = out.join("moddef_descriptor.bin");
    std::fs::write(&descriptor_path, prost::Message::encode_to_vec(&fds))?;

    prost_build::Config::new()
        .btree_map(["."])
        .file_descriptor_set_path(&descriptor_path)
        .skip_protoc_run()
        .compile_protos(&files.map(PathBuf::from), &[PathBuf::from("proto")])?;

    pbjson_build::Builder::new()
        .register_descriptors(&std::fs::read(&descriptor_path)?)?
        .build(&[".moddef.v1"])?;

    Ok(())
}
