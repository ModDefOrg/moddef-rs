// SPDX-License-Identifier: Apache-2.0

//! Device facade tests (spec §32.4, §26) mirroring moddef-ts device.test.ts:
//! point reads across spaces, measurand queries with ambiguity handling,
//! SunSpec discovery with ID-relative model offsets (spec §7.3), scale_ref
//! companion reads, and constrained writes — over an in-memory transport.

mod common;

use common::MockTransport;
use moddef_core::{
    parse_document, ConstraintKind, DecodedValue, Device, DocumentFormat, Error, MeasurandQuery,
    Value,
};

const METER_DOC: &str = r#"{
  "docId": "test.meter",
  "version": "1.0.0",
  "devices": [{
    "deviceId": "meter",
    "vendor": "Test",
    "model": "M1",
    "blocks": [
      {
        "blockId": "live",
        "space": "INPUT_REGISTER",
        "startOffset": 0,
        "lengthWords": 16,
        "points": [
          {
            "pointId": "voltage_l1",
            "name": "Voltage L1",
            "access": "READ_ONLY",
            "storageType": "U16",
            "valueType": {"primitive": "DECIMAL"},
            "unit": "V",
            "mapping": {"space": "INPUT_REGISTER", "offset": 0, "lengthWords": 1},
            "transform": {"scale": {"numerator": "1", "denominator": "10"}},
            "measurand": {"baseQuantity": "voltage", "phaseRef": "L1_N"}
          },
          {
            "pointId": "voltage_l2",
            "name": "Voltage L2",
            "access": "READ_ONLY",
            "storageType": "U16",
            "valueType": {"primitive": "DECIMAL"},
            "unit": "V",
            "mapping": {"space": "INPUT_REGISTER", "offset": 1, "lengthWords": 1},
            "transform": {"scale": {"numerator": "1", "denominator": "10"}},
            "measurand": {"baseQuantity": "voltage", "phaseRef": "L2_N"},
            "naValues": [{"raw": "65535", "meaning": "not_implemented"}]
          },
          {
            "pointId": "frequency",
            "name": "Frequency",
            "access": "READ_ONLY",
            "storageType": "U16",
            "valueType": {"primitive": "DECIMAL"},
            "unit": "Hz",
            "mapping": {"space": "INPUT_REGISTER", "offset": 2, "lengthWords": 1},
            "transform": {"scale": {"numerator": "1", "denominator": "100"}},
            "measurand": {"baseQuantity": "frequency"}
          }
        ]
      },
      {
        "blockId": "settings",
        "space": "HOLDING_REGISTER",
        "startOffset": 0,
        "lengthWords": 8,
        "points": [
          {
            "pointId": "stop_soc",
            "name": "Stop SOC",
            "access": "READ_WRITE",
            "storageType": "U16",
            "valueType": {"primitive": "DECIMAL"},
            "unit": "%",
            "mapping": {"space": "HOLDING_REGISTER", "offset": 0, "lengthWords": 1},
            "write": {
              "behavior": "DIRECT",
              "constraints": {
                "minValue": {"numerator": "0", "denominator": "1"},
                "maxValue": {"numerator": "100", "denominator": "1"},
                "step": {"numerator": "1", "denominator": "1"}
              }
            }
          },
          {
            "pointId": "mode",
            "name": "Mode",
            "access": "READ_WRITE",
            "storageType": "U16",
            "valueType": {"primitive": "UINT32"},
            "mapping": {"space": "HOLDING_REGISTER", "offset": 1, "lengthWords": 1},
            "write": {"behavior": "DIRECT", "constraints": {"allowedValues": ["0", "1", "2"]}}
          },
          {
            "pointId": "setpoint_scaled",
            "name": "Scaled Setpoint",
            "access": "READ_WRITE",
            "storageType": "U16",
            "valueType": {"primitive": "DECIMAL"},
            "mapping": {"space": "HOLDING_REGISTER", "offset": 2, "lengthWords": 1},
            "transform": {"scale": {"numerator": "1", "denominator": "10"}},
            "write": {"behavior": "DIRECT"}
          }
        ]
      }
    ]
  }]
}"#;

fn f64_of(v: DecodedValue) -> f64 {
    v.as_f64()
        .unwrap_or_else(|| panic!("expected numeric, got {v:?}"))
}

#[tokio::test]
async fn reads_and_scales_input_registers() {
    let doc = parse_document(METER_DOC.as_bytes(), DocumentFormat::Json).unwrap();
    let mut m = MockTransport::new(64);
    m.input[0] = 2305;
    let mut dev = Device::new(&doc, Some("meter"), m).unwrap();
    assert!((f64_of(dev.read_point("voltage_l1").await.unwrap()) - 230.5).abs() < 1e-10);
}

#[tokio::test]
async fn sentinel_returns_unavailable_with_meaning() {
    let doc = parse_document(METER_DOC.as_bytes(), DocumentFormat::Json).unwrap();
    let mut m = MockTransport::new(64);
    m.input[1] = 0xffff;
    let mut dev = Device::new(&doc, Some("meter"), m).unwrap();
    assert_eq!(
        dev.read_point("voltage_l2").await.unwrap(),
        DecodedValue::Unavailable("not_implemented".into())
    );
}

#[tokio::test]
async fn unknown_point_id() {
    let doc = parse_document(METER_DOC.as_bytes(), DocumentFormat::Json).unwrap();
    let mut dev = Device::new(&doc, Some("meter"), MockTransport::new(64)).unwrap();
    assert!(matches!(
        dev.read_point("nope").await,
        Err(Error::PointNotFound)
    ));
}

#[tokio::test]
async fn measurand_queries() {
    let doc = parse_document(METER_DOC.as_bytes(), DocumentFormat::Json).unwrap();
    let mut m = MockTransport::new(64);
    m.input[0] = 2301;
    m.input[2] = 4999;
    let mut dev = Device::new(&doc, Some("meter"), m).unwrap();

    // Unqualified unique match.
    let v = dev
        .read_measurand(&MeasurandQuery::base("frequency"))
        .await
        .unwrap();
    assert!((f64_of(v) - 49.99).abs() < 1e-10);

    // Qualified phase match.
    let q = MeasurandQuery::base("voltage").phase(moddef_core::schema::PhaseRef::L1N);
    assert!((f64_of(dev.read_measurand(&q).await.unwrap()) - 230.1).abs() < 1e-10);

    // Ambiguous and unsupported.
    assert!(matches!(
        dev.read_measurand(&MeasurandQuery::base("voltage")).await,
        Err(Error::AmbiguousMeasurand)
    ));
    assert!(matches!(
        dev.read_measurand(&MeasurandQuery::base("battery_power"))
            .await,
        Err(Error::MeasurandNotSupported)
    ));
}

#[tokio::test]
async fn writes_with_constraints() {
    let doc = parse_document(METER_DOC.as_bytes(), DocumentFormat::Json).unwrap();
    let mut dev = Device::new(&doc, Some("meter"), MockTransport::new(64)).unwrap();

    dev.write_point("stop_soc", Value::F64(80.0)).await.unwrap();
    assert_eq!(dev.transport_mut().holding[0], 80);

    for (v, kind) in [
        (101.0, ConstraintKind::Max),
        (-1.0, ConstraintKind::Min),
        (50.5, ConstraintKind::Step),
    ] {
        match dev.write_point("stop_soc", Value::F64(v)).await {
            Err(Error::WriteConstraint(k)) => assert_eq!(k, kind),
            other => panic!("expected constraint {kind:?}, got {other:?}"),
        }
    }

    dev.write_point("mode", Value::U64(2)).await.unwrap();
    assert_eq!(dev.transport_mut().holding[1], 2);
    assert!(matches!(
        dev.write_point("mode", Value::U64(3)).await,
        Err(Error::WriteConstraint(ConstraintKind::AllowedValues))
    ));

    // Inverse transform on write.
    dev.write_point("setpoint_scaled", Value::F64(23.5))
        .await
        .unwrap();
    assert_eq!(dev.transport_mut().holding[2], 235);

    // Read-only points are rejected.
    assert!(matches!(
        dev.write_point("voltage_l1", Value::F64(1.0)).await,
        Err(Error::WriteAccess)
    ));
}

const SUNSPEC_DOC: &str = r#"{
  "docId": "test.sunspec",
  "version": "1.0.0",
  "devices": [{
    "deviceId": "inv",
    "vendor": "Test",
    "model": "S1",
    "blocks": [{
      "blockId": "inverter",
      "space": "HOLDING_REGISTER",
      "startOffset": 40070,
      "lengthWords": 50,
      "discovery": {"kind": "SUNSPEC", "anchorCandidates": [0, 40000, 50000], "modelId": 103},
      "points": [
        {
          "pointId": "w_sf",
          "name": "Power SF",
          "access": "READ_ONLY",
          "storageType": "S16",
          "valueType": {"primitive": "INT32"},
          "mapping": {"space": "HOLDING_REGISTER", "modelRelativeOffset": 15, "lengthWords": 1}
        },
        {
          "pointId": "ac_power",
          "name": "AC Power",
          "access": "READ_ONLY",
          "storageType": "S16",
          "valueType": {"primitive": "DECIMAL"},
          "unit": "W",
          "mapping": {"space": "HOLDING_REGISTER", "modelRelativeOffset": 14, "lengthWords": 1},
          "transform": {"scaleRef": {"pointId": "w_sf", "mode": "POW10"}},
          "measurand": {"baseQuantity": "active_power"}
        }
      ]
    }]
  }]
}"#;

#[tokio::test]
async fn sunspec_discovery_id_relative_offsets() {
    let doc = parse_document(SUNSPEC_DOC.as_bytes(), DocumentFormat::Json).unwrap();
    let mut m = MockTransport::new(41000);
    m.holding[40000] = 0x5375; // "Su"
    m.holding[40001] = 0x6e53; // "nS"
    m.holding[40002] = 1; //      model 1 header
    m.holding[40003] = 66;
    m.holding[40070] = 103; //    model 103 header (= 40002 + 2 + 66)
    m.holding[40071] = 50;
    // Canonical model 103: W at ID+14, W_SF at ID+15.
    m.holding[40070 + 14] = 2301;
    m.holding[40070 + 15] = 0xffff; // sf = -1
    m.holding[40122] = 0xffff; //     end of chain

    let mut dev = Device::new(&doc, Some("inv"), m).unwrap();
    assert!((f64_of(dev.read_point("ac_power").await.unwrap()) - 230.1).abs() < 1e-10);

    // Model base is cached: a second read must not re-probe the anchor.
    let probes = |t: &MockTransport| t.read_log.iter().filter(|l| *l == "H@40000x2").count();
    let before = probes(dev.transport_mut());
    dev.read_point("ac_power").await.unwrap();
    assert_eq!(probes(dev.transport_mut()), before);

    // The measurand path resolves through discovery too.
    let v = dev
        .read_measurand(&MeasurandQuery::base("active_power"))
        .await
        .unwrap();
    assert!((f64_of(v) - 230.1).abs() < 1e-10);
}
