//! Generated-client conformance (spec §31): build.rs runs moddef-codegen
//! over every blessed registry profile plus the SunSpec golden fixture and
//! this file includes the emitted modules — a full compile gate — then
//! drives two of the generated clients end-to-end over the mock transport.

mod common;

// One `pub mod <doc> { include!(...gen/<doc>.rs) }` per generated profile.
include!(concat!(env!("OUT_DIR"), "/gen/mods.rs"));

use common::MockTransport;
use moddef_core::{ConstraintKind, Error, Reading};

use growatt_sph::{GridFirstSlot1Start, GrowattSph, InverterRunState};

#[test]
fn descriptor_tables_are_populated() {
    assert_eq!(growatt_sph::POINTS.len(), 396, "growatt full coverage");
    assert!(!example_sunspec_inverter::POINTS.is_empty());
}

#[tokio::test]
async fn growatt_enum_scaling_fields_and_writes() {
    let mut m = MockTransport::new(2048);
    m.input[0] = 1; //                       inverter_status = NORMAL
    m.input[3] = 2305; //                    pv1_voltage raw (x0.1)
    m.holding[1080] = (21 << 8) | 45; //     grid_first_slot1_start 21:45
    let mut dev = GrowattSph::new(m);

    assert_eq!(
        dev.inverter_status().await.unwrap(),
        InverterRunState::Normal
    );

    // Lossless unknown enum values (§12).
    dev.transport.input[0] = 99;
    assert_eq!(
        dev.inverter_status().await.unwrap(),
        InverterRunState::Unknown(99)
    );

    let v = dev.pv1_voltage().await.unwrap();
    assert!((v - 230.5).abs() < 1e-10);

    // Packed time-slot register decodes into the generated field struct (§13).
    assert_eq!(
        dev.grid_first_slot1_start().await.unwrap(),
        GridFirstSlot1Start {
            hour: 21,
            minute: 45
        }
    );

    // §11.4-validated write path.
    dev.set_active_power_rate(50.0).await.unwrap();
    assert_eq!(dev.transport.holding[3], 50);
    assert!(matches!(
        dev.set_active_power_rate(300.0).await,
        Err(Error::WriteConstraint(ConstraintKind::Max))
    ));
}

#[tokio::test]
async fn sunspec_generated_discovery_and_scale_ref() {
    let mut m = MockTransport::new(41000);
    m.holding[40000] = 0x5375; // "Su"
    m.holding[40001] = 0x6e53; // "nS"
    m.holding[40002] = 1; //      model 1 header
    m.holding[40003] = 66;
    m.holding[40070] = 103; //    model 103 header (= 40002 + 2 + 66)
    m.holding[40071] = 52;
    m.holding[40070 + 14] = 2301; //  W
    m.holding[40070 + 15] = 0xffff; // W_SF = -1
    m.holding[40124] = 0xffff; //     end of chain

    let mut dev = example_sunspec_inverter::Inv::new(m);
    match dev.ac_power().await.unwrap() {
        Reading::Value(v) => assert!((v - 230.1).abs() < 1e-10),
        other => panic!("expected value, got {other:?}"),
    }

    // Model base is cached: a second read must not re-probe the anchor.
    let probes = |t: &MockTransport| t.read_log.iter().filter(|l| *l == "H@40000x2").count();
    let before = probes(&dev.transport);
    dev.ac_power().await.unwrap();
    assert_eq!(probes(&dev.transport), before);

    // §8.4 sentinel surfaces as Reading::Unavailable with the meaning.
    dev.transport.holding[40070 + 14] = 0x8000;
    assert_eq!(
        dev.ac_power().await.unwrap(),
        Reading::Unavailable("not_implemented")
    );
}
