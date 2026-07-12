// SPDX-License-Identifier: Apache-2.0

//! Host-side codegen: parse the Growatt SPH registry profile and emit the
//! typed client into OUT_DIR; the no_std lib includes it.

fn main() {
    let profile = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../devices/solar-inverter/growatt-sph/growatt-sph.moddef.yaml");
    println!("cargo:rerun-if-changed={}", profile.display());
    let doc = moddef_core::load(&profile).expect("parse profile");
    let out = std::path::PathBuf::from(std::env::var("OUT_DIR").unwrap());
    for g in moddef_codegen::generate(&doc) {
        std::fs::write(out.join(&g.path), &g.content).unwrap();
    }
}
