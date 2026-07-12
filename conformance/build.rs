// SPDX-License-Identifier: Apache-2.0

//! Compile gate for the generator (spec §31): generate typed clients for
//! every blessed registry profile plus the SunSpec golden fixture into
//! OUT_DIR; tests/generated.rs includes them all as modules, so `cargo test`
//! fails if any emitted client stops compiling.

use std::path::Path;

const PROFILES: &[&str] = &[
    "devices/solar-inverter/growatt-sph/growatt-sph.moddef.yaml",
    "devices/solar-inverter/fronius-gen24/fronius-gen24.moddef.yaml",
    "devices/energy-meter/eastron-sdm630/eastron-sdm630.moddef.yaml",
    "devices/energy-meter/abb-b23/abb-b23.moddef.yaml",
    "devices/energy-meter/carlo-gavazzi-em24/carlo-gavazzi-em24.moddef.yaml",
    "devices/battery-storage/victron-venus-os/victron-venus-os.moddef.yaml",
    "devices/ev-charger/abb-terra-ac/abb-terra-ac.moddef.yaml",
    "devices/hvac/daikin-altherma-3/daikin-altherma-3.moddef.yaml",
    "moddef/fixtures/golden/sunspec-inverter/sunspec-inverter.moddef.yaml",
];

fn main() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let out = std::path::PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("gen");
    std::fs::create_dir_all(&out).unwrap();

    let mut mods = String::new();
    for rel in PROFILES {
        let path = root.join(rel);
        println!("cargo:rerun-if-changed={}", path.display());
        let doc = moddef_core::load(&path).unwrap_or_else(|e| panic!("{rel}: {e}"));
        for g in moddef_codegen::generate(&doc) {
            std::fs::write(out.join(&g.path), &g.content).unwrap();
            let name = g.path.trim_end_matches(".rs");
            mods.push_str(&format!(
                "#[allow(clippy::all, dead_code, unused_imports, unused_variables)]\n\
                 pub mod {name} {{ include!(concat!(env!(\"OUT_DIR\"), \"/gen/{}\")); }}\n",
                g.path
            ));
        }
    }
    std::fs::write(out.join("mods.rs"), mods).unwrap();
}
