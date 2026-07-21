// SPDX-License-Identifier: Apache-2.0

//! Command executor tests (spec §11.7) mirroring the Go/TS/Python suites:
//! linear step order, param/trigger writes, poll conditions on raw values
//! with (delay-accumulated) timeout, length_ref-sized reads, chunked write
//! transfers, and result assembly from bindings.

mod common;

use std::collections::HashMap;

use common::{MockTransport, OutOfRange};
use moddef_core::{
    condition_met, parse_document, schema, DecodedValue, Delay, Device, DocumentFormat, Error,
    Transport,
};

const CMD_DOC: &str = r#"{
  "docId": "test.commands",
  "version": "1.0.0",
  "devices": [{
    "deviceId": "cmd-device",
    "vendor": "Test",
    "model": "CMD-1",
    "blocks": [
      {
        "blockId": "job",
        "space": "HOLDING_REGISTER",
        "startOffset": 0,
        "lengthWords": 1000,
        "points": [
          {
            "pointId": "control",
            "access": "COMMAND",
            "storageType": "U16",
            "valueType": {"primitive": "UINT32"},
            "mapping": {"space": "HOLDING_REGISTER", "offset": 10, "lengthWords": 1},
            "write": {"behavior": "COMMAND_TRIGGER"}
          },
          {
            "pointId": "status",
            "access": "READ_ONLY",
            "storageType": "U16",
            "valueType": {"primitive": "UINT32"},
            "mapping": {"space": "HOLDING_REGISTER", "offset": 11, "lengthWords": 1}
          },
          {
            "pointId": "busy_flags",
            "access": "READ_ONLY",
            "storageType": "U16",
            "valueType": {"primitive": "UINT32"},
            "mapping": {"space": "HOLDING_REGISTER", "offset": 12, "lengthWords": 1}
          },
          {
            "pointId": "result_length",
            "access": "READ_ONLY",
            "storageType": "U16",
            "valueType": {"primitive": "UINT32"},
            "mapping": {"space": "HOLDING_REGISTER", "offset": 20, "lengthWords": 1}
          },
          {
            "pointId": "result_data",
            "access": "READ_ONLY",
            "storageType": "BYTES_RAW",
            "valueType": {"primitive": "BYTES"},
            "mapping": {
              "space": "HOLDING_REGISTER", "offset": 21, "lengthWords": 8,
              "byteOrder": "BIG_ENDIAN", "wordOrder": "WORD_BIG_ENDIAN",
              "lengthRef": {"pointId": "result_length"}
            }
          },
          {
            "pointId": "blob",
            "access": "READ_ONLY",
            "storageType": "BYTES_RAW",
            "valueType": {"primitive": "BYTES"},
            "mapping": {"space": "HOLDING_REGISTER", "offset": 500, "lengthWords": 300}
          }
        ]
      }
    ],
    "commands": [
      {
        "commandId": "run_job",
        "params": [
          {
            "field": "payload",
            "storageType": "BYTES_RAW",
            "valueType": {"primitive": "BYTES"},
            "mapping": {"space": "HOLDING_REGISTER", "offset": 0, "lengthWords": 4}
          },
          {
            "field": "mode",
            "storageType": "U16",
            "valueType": {"primitive": "UINT32"},
            "mapping": {"space": "HOLDING_REGISTER", "offset": 5, "lengthWords": 1},
            "required": true
          }
        ],
        "steps": [
          {"name": "write_payload", "write": {"param": "payload"}},
          {"name": "write_mode", "write": {"param": "mode"}},
          {"name": "arm", "write": {"trigger": {"pointId": "control", "value": "1"}}},
          {"name": "wait_not_busy",
           "poll": {"pointId": "busy_flags",
                    "until": {"op": "MASK", "mask": "1", "value": "0"},
                    "intervalMs": 2, "timeoutMs": 500}},
          {"name": "wait_done",
           "poll": {"pointId": "status",
                    "until": {"op": "EQ", "value": "0"},
                    "intervalMs": 2, "timeoutMs": 500}},
          {"name": "fetch_length", "read": {"pointId": "result_length", "into": "length"}},
          {"name": "fetch_data", "read": {"pointId": "result_data", "into": "data"}}
        ],
        "results": [
          {"field": "data", "from": "data", "valueType": {"primitive": "BYTES"}},
          {"field": "length", "from": "length", "valueType": {"primitive": "UINT32"}}
        ]
      },
      {
        "commandId": "wait_forever",
        "steps": [
          {"name": "poll",
           "poll": {"pointId": "status",
                    "until": {"op": "EQ", "value": "9"},
                    "intervalMs": 2, "timeoutMs": 20}}
        ]
      },
      {
        "commandId": "xfer",
        "params": [
          {
            "field": "input",
            "storageType": "BYTES_RAW",
            "valueType": {"primitive": "BYTES"},
            "mapping": {"space": "HOLDING_REGISTER", "offset": 600, "lengthWords": 200},
            "required": true
          }
        ],
        "steps": [
          {"name": "w", "write": {"param": "input"}},
          {"name": "r", "read": {"pointId": "blob", "into": "blob"}}
        ],
        "results": [{"field": "blob", "from": "blob"}]
      }
    ]
  }]
}"#;

/// MockTransport wrapper with per-offset successive reads and a write log.
struct CmdTransport {
    inner: MockTransport,
    seq: HashMap<u16, Vec<u16>>,
    writes: Vec<(u16, Vec<u16>)>,
}

impl CmdTransport {
    fn new() -> Self {
        CmdTransport {
            inner: MockTransport::new(1024),
            seq: HashMap::new(),
            writes: Vec::new(),
        }
    }
}

impl Transport for CmdTransport {
    type Error = OutOfRange;

    async fn read_holding(&mut self, offset: u16, out: &mut [u16]) -> Result<(), OutOfRange> {
        if let Some(vals) = self.seq.get_mut(&offset) {
            if !vals.is_empty() {
                let v = if vals.len() > 1 {
                    vals.remove(0)
                } else {
                    vals[0]
                };
                out[0] = v;
                return Ok(());
            }
        }
        self.inner.read_holding(offset, out).await
    }

    async fn read_input(&mut self, offset: u16, out: &mut [u16]) -> Result<(), OutOfRange> {
        self.inner.read_input(offset, out).await
    }

    async fn read_coils(&mut self, offset: u16, out: &mut [bool]) -> Result<(), OutOfRange> {
        self.inner.read_coils(offset, out).await
    }

    async fn read_discrete(&mut self, offset: u16, out: &mut [bool]) -> Result<(), OutOfRange> {
        self.inner.read_discrete(offset, out).await
    }

    async fn write_holding(&mut self, offset: u16, regs: &[u16]) -> Result<(), OutOfRange> {
        self.writes.push((offset, regs.to_vec()));
        self.inner.write_holding(offset, regs).await
    }

    async fn write_coil(&mut self, offset: u16, on: bool) -> Result<(), OutOfRange> {
        self.inner.write_coil(offset, on).await
    }
}

/// Instant "delay" that only counts requested milliseconds — the executor
/// accounts poll timeouts by accumulation, so tests run with no real sleeps.
#[derive(Default)]
struct CountingDelay {
    total_ms: u64,
}

impl Delay for CountingDelay {
    async fn delay_ms(&mut self, ms: u32) {
        self.total_ms += u64::from(ms);
    }
}

fn doc() -> schema::ModDefDocument {
    parse_document(CMD_DOC.as_bytes(), DocumentFormat::Json).expect("parse")
}

#[tokio::test]
async fn run_command_full_cycle() {
    let doc = doc();
    let mut t = CmdTransport::new();
    t.seq.insert(12, vec![1, 0]); // busy clears on the second poll
    t.seq.insert(11, vec![5, 0]); // status goes 0 on the second poll
    t.inner.holding[20] = 2; // result_length: 2 of the 8-word window
    t.inner.holding[21] = 0xDEAD;
    t.inner.holding[22] = 0xBEEF;
    t.inner.holding[23] = 0xFFFF; // beyond the live length; must not be included

    let mut dev = Device::new(&doc, Some("cmd-device"), t).unwrap();
    let mut delay = CountingDelay::default();
    let payload: &[u8] = &[1, 2, 3, 4];
    let out = dev
        .run_command(
            "run_job",
            &[("mode", 7i64.into()), ("payload", payload.into())],
            &mut delay,
        )
        .await
        .expect("run_command");

    let t = dev.into_transport();
    // Step wire order: payload @0, mode @5, trigger @10.
    let offsets: Vec<u16> = t.writes.iter().map(|w| w.0).collect();
    assert_eq!(offsets, vec![0, 5, 10]);
    assert_eq!(t.writes[0].1, vec![0x0102, 0x0304, 0, 0]);
    assert_eq!(t.writes[1].1, vec![7]);
    assert_eq!(t.writes[2].1, vec![1]);

    // length_ref-sized read: 2 words -> 4 bytes, not the 8-word clamp.
    assert_eq!(
        out.get("data"),
        Some(&DecodedValue::Bytes(vec![0xDE, 0xAD, 0xBE, 0xEF]))
    );
    assert_eq!(out.get("length"), Some(&DecodedValue::U64(2)));
}

#[tokio::test]
async fn read_point_honours_length_ref() {
    let doc = doc();
    let mut t = CmdTransport::new();
    t.inner.holding[20] = 3;
    t.inner.holding[21] = 0x0102;
    t.inner.holding[22] = 0x0304;
    t.inner.holding[23] = 0x0506;
    let mut dev = Device::new(&doc, Some("cmd-device"), t).unwrap();
    let v = dev.read_point("result_data").await.unwrap();
    assert_eq!(v, DecodedValue::Bytes(vec![1, 2, 3, 4, 5, 6]));
}

#[tokio::test]
async fn run_command_errors() {
    let doc = doc();
    let mut dev = Device::new(&doc, Some("cmd-device"), CmdTransport::new()).unwrap();
    let mut delay = CountingDelay::default();

    match dev.run_command("nope", &[], &mut delay).await {
        Err(Error::CommandNotFound) => {}
        other => panic!("expected CommandNotFound, got {other:?}"),
    }
    let payload: &[u8] = &[1];
    match dev
        .run_command("run_job", &[("payload", payload.into())], &mut delay)
        .await
    {
        Err(Error::RequiredParamMissing) => {}
        other => panic!("expected RequiredParamMissing, got {other:?}"),
    }
}

#[tokio::test]
async fn poll_timeout_accumulates_delay() {
    let doc = doc();
    let mut dev = Device::new(&doc, Some("cmd-device"), CmdTransport::new()).unwrap();
    let mut delay = CountingDelay::default();
    match dev.run_command("wait_forever", &[], &mut delay).await {
        Err(Error::PollTimeout) => {}
        other => panic!("expected PollTimeout, got {other:?}"),
    }
    // 20ms timeout at 2ms intervals: exactly the timeout budget was slept.
    assert_eq!(delay.total_ms, 20);
}

#[tokio::test]
async fn chunked_write_transfers() {
    let doc = doc();
    let mut dev = Device::new(&doc, Some("cmd-device"), CmdTransport::new()).unwrap();
    let mut delay = CountingDelay::default();
    let input = vec![0u8; 400];
    let out = dev
        .run_command("xfer", &[("input", input.as_slice().into())], &mut delay)
        .await
        .expect("xfer");

    let t = dev.into_transport();
    // 200-word write in <=123-word chunks: 123 + 77; second at offset 600+123.
    let sizes: Vec<usize> = t.writes.iter().map(|w| w.1.len()).collect();
    assert_eq!(sizes, vec![123, 77]);
    assert_eq!(t.writes[1].0, 600 + 123);
    // 300-word read reassembled: 600 bytes.
    match out.get("blob") {
        Some(DecodedValue::Bytes(b)) => assert_eq!(b.len(), 600),
        other => panic!("blob = {other:?}"),
    }
}

#[test]
fn condition_ops() {
    use schema::{Condition, ConditionOp};
    let c = |op: ConditionOp, value: i64, mask: i64, min: i64, max: i64| Condition {
        op: op as i32,
        value,
        mask,
        min,
        max,
    };
    assert!(condition_met(Some(&c(ConditionOp::Eq, 5, 0, 0, 0)), 5));
    assert!(!condition_met(Some(&c(ConditionOp::Eq, 5, 0, 0, 0)), 4));
    assert!(condition_met(Some(&c(ConditionOp::Ne, 5, 0, 0, 0)), 4));
    assert!(!condition_met(Some(&c(ConditionOp::Ne, 5, 0, 0, 0)), 5));
    assert!(condition_met(
        Some(&c(ConditionOp::Mask, 0x03, 0x0F, 0, 0)),
        0xF3
    ));
    assert!(!condition_met(
        Some(&c(ConditionOp::Mask, 0x03, 0x0F, 0, 0)),
        0xF4
    ));
    assert!(condition_met(
        Some(&c(ConditionOp::Range, 0, 0, 10, 20)),
        15
    ));
    assert!(!condition_met(
        Some(&c(ConditionOp::Range, 0, 0, 10, 20)),
        21
    ));
    assert!(!condition_met(None, 0));
}
