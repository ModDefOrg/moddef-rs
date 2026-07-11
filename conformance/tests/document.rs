//! Fixture conformance (spec §33): YAML / JSON / binary triples must parse to
//! equal documents and round-trip losslessly; binary serialization must
//! byte-match the checked-in goldens (deterministic BTreeMap encoding);
//! schema-invalid documents must be rejected at parse time; and every blessed
//! registry profile must parse. Mirrors moddef-ts conformance/document.test.ts.

use std::path::{Path, PathBuf};

use moddef_core::{parse_document, serialize_document, DocumentFormat};
use serde::Deserialize;

fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../moddef/fixtures")
}

fn devices_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../devices")
}

#[derive(Deserialize)]
struct Manifest {
    golden: Vec<Golden>,
    invalid: Vec<Invalid>,
}

#[derive(Deserialize)]
struct Golden {
    name: String,
    files: GoldenFiles,
}

#[derive(Deserialize)]
struct GoldenFiles {
    json: String,
    yaml: String,
    binary: String,
}

#[derive(Deserialize)]
struct Invalid {
    rule: String,
    file: String,
    schema_valid: bool,
}

fn load_manifest() -> Manifest {
    let raw = std::fs::read_to_string(fixtures_dir().join("manifest.yaml")).unwrap();
    serde_yaml::from_str(&raw).unwrap()
}

fn read(rel: &str) -> Vec<u8> {
    std::fs::read(fixtures_dir().join(rel)).unwrap()
}

#[test]
fn golden_yaml_json_binary_equivalence() {
    for g in load_manifest().golden {
        let yaml = parse_document(&read(&g.files.yaml), DocumentFormat::Yaml)
            .unwrap_or_else(|e| panic!("{}: yaml: {e}", g.name));
        let json = parse_document(&read(&g.files.json), DocumentFormat::Json)
            .unwrap_or_else(|e| panic!("{}: json: {e}", g.name));
        let bin = parse_document(&read(&g.files.binary), DocumentFormat::Binary)
            .unwrap_or_else(|e| panic!("{}: binary: {e}", g.name));

        assert_eq!(yaml, json, "{}: yaml != json", g.name);
        assert_eq!(yaml, bin, "{}: yaml != binary", g.name);
    }
}

#[test]
fn golden_lossless_round_trips() {
    for g in load_manifest().golden {
        let doc = parse_document(&read(&g.files.yaml), DocumentFormat::Yaml).unwrap();

        for format in [
            DocumentFormat::Json,
            DocumentFormat::Yaml,
            DocumentFormat::Binary,
        ] {
            let bytes = serialize_document(&doc, format).unwrap();
            let round = parse_document(&bytes, format)
                .unwrap_or_else(|e| panic!("{}: round-trip {format:?}: {e}", g.name));
            assert_eq!(doc, round, "{}: lossy round-trip via {format:?}", g.name);
        }

        // Binary equivalence with the checked-in .moddef bytes. One prost
        // quirk: Go/protobuf-es always emit map-entry key/value fields, prost
        // omits them when zero — wire-equivalent, but not byte-identical. So
        // for documents with a zero map key (e.g. a flag on bit 0), compare
        // against the golden re-encoded by prost instead of the raw bytes.
        let golden_bytes = read(&g.files.binary);
        let expected = if has_zero_map_key(&doc) {
            let golden = parse_document(&golden_bytes, DocumentFormat::Binary).unwrap();
            serialize_document(&golden, DocumentFormat::Binary).unwrap()
        } else {
            golden_bytes
        };
        assert_eq!(
            serialize_document(&doc, DocumentFormat::Binary).unwrap(),
            expected,
            "{}: binary bytes differ from golden",
            g.name
        );
    }
}

/// Does any FlagSet bit table or selector_ref case map use key 0? (The two
/// map fields in the schema.)
fn has_zero_map_key(doc: &moddef_core::schema::ModDefDocument) -> bool {
    use moddef_core::schema::{value_type::Kind, Point};
    let point_has = |p: &Point| {
        let flags_zero = matches!(
            p.value_type.as_ref().and_then(|v| v.kind.as_ref()),
            Some(Kind::Flags(fl)) if fl.bits.contains_key(&0)
        );
        flags_zero
            || p.selector_ref
                .as_ref()
                .is_some_and(|s| s.cases.contains_key(&0))
    };
    doc.devices.iter().any(|d| {
        d.blocks.iter().any(|b| b.points.iter().any(point_has))
            || d.variants.iter().any(|v| {
                v.additions.iter().any(point_has)
                    || v.overrides
                        .iter()
                        .any(|o| o.replacement.as_ref().is_some_and(point_has))
            })
    })
}

#[test]
fn schema_invalid_fixtures_are_rejected() {
    for inv in load_manifest().invalid.iter().filter(|i| !i.schema_valid) {
        let raw = read(&inv.file);
        assert!(
            parse_document(&raw, DocumentFormat::Json).is_err(),
            "{}: expected a parse error",
            inv.rule
        );
    }
}

#[test]
fn blessed_registry_profiles_parse() {
    let profiles = [
        "solar-inverter/growatt-sph/growatt-sph.moddef.yaml",
        "solar-inverter/fronius-gen24/fronius-gen24.moddef.yaml",
        "energy-meter/eastron-sdm630/eastron-sdm630.moddef.yaml",
        "energy-meter/abb-b23/abb-b23.moddef.yaml",
        "energy-meter/carlo-gavazzi-em24/carlo-gavazzi-em24.moddef.yaml",
        "battery-storage/victron-venus-os/victron-venus-os.moddef.yaml",
        "ev-charger/abb-terra-ac/abb-terra-ac.moddef.yaml",
        "hvac/daikin-altherma-3/daikin-altherma-3.moddef.yaml",
    ];
    for rel in profiles {
        let raw = std::fs::read(devices_dir().join(rel)).unwrap();
        let doc =
            parse_document(&raw, DocumentFormat::Yaml).unwrap_or_else(|e| panic!("{rel}: {e}"));
        assert!(!doc.doc_id.is_empty(), "{rel}: empty doc_id");
        assert!(!doc.devices.is_empty(), "{rel}: no devices");

        // JSON round-trip is lossless for real-world profiles too.
        let bytes = serialize_document(&doc, DocumentFormat::Json).unwrap();
        let round = parse_document(&bytes, DocumentFormat::Json).unwrap();
        assert_eq!(doc, round, "{rel}: lossy json round-trip");
    }
}
